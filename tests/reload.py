"""Transactional route reloads, worker draining and real live-session continuity."""
from __future__ import annotations
import argparse
import asyncio
import json
import os
import signal
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from reconnect import Wire

ROOT = Path(__file__).resolve().parents[1]


async def exercise(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-reload-') as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        config = root / 'harbor.toml'
        base_config = f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
        app = f"[[apps]]\nname='fixture'\nmount='/fixture'\npath={json.dumps(str(ROOT / 'examples/compatibility'))}\n"
        extra = f"[[apps]]\nname='added'\nmount='/added'\npath={json.dumps(str(ROOT / 'examples/hello'))}\n"
        original = base_config + app
        config.write_text(original, encoding='utf-8')
        with (results / 'reload.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                token = (root / '.harbor/admin.token').read_text()
                base = f'http://127.0.0.1:{public}'
                async with ClientSession() as client, async_playwright() as p:
                    async def reload(status=200):
                        async with client.post(f'http://127.0.0.1:{admin}/api/reload', headers={'Authorization': 'Bearer ' + token}) as response:
                            assert response.status == status, await response.text()
                    browser = await p.chromium.launch(headless=True)
                    try:
                        context = await browser.new_context()
                        page = await context.new_page()
                        await page.goto(base + '/fixture/')
                        await expect(page.locator('#double')).to_have_text('4', timeout=30000)
                        pid = await page.locator('#pid').inner_text()
                        session = await page.evaluate('Shiny.shinyapp.config.sessionId')
                        async with client.post(f'http://127.0.0.1:{admin}/api/reload') as response:
                            assert response.status == 401
                        checks.append('reload-requires-admin-token')
                        config.write_text(original + extra, encoding='utf-8')
                        await reload()
                        async with client.get(base + '/added/') as response:
                            assert response.status == 200
                        await page.reload()
                        await expect(page.locator('#pid')).to_have_text(pid, timeout=30000)
                        session = await page.evaluate('Shiny.shinyapp.config.sessionId')
                        checks.append('added-route-and-unchanged-worker-reuse')
                        # Changed settings create a new generation; old tabs and
                        # their session-specific uploads/downloads must still work.
                        config.write_text(original + "label='Changed application'\n" + extra, encoding='utf-8')
                        command = await asyncio.create_subprocess_exec(str(binary.resolve()), 'reload', '--config', str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                        stdout, stderr = await command.communicate()
                        assert command.returncode == 0, (stdout, stderr)
                        new_page = await context.new_page()
                        await new_page.goto(base + '/fixture/')
                        await expect(new_page.locator('#pid')).not_to_have_text(pid, timeout=30000)
                        await expect(new_page.locator('#double')).to_have_text('4')
                        await page.locator('#n').fill('29')
                        await page.locator('#n').dispatch_event('change')
                        await expect(page.locator('#double')).to_have_text('58')
                        assert await page.evaluate('Shiny.shinyapp.config.sessionId') == session
                        await page.locator('#upload').set_input_files({'name':'old.txt', 'mimeType':'text/plain', 'buffer':b'old session upload'})
                        await expect(page.locator('#uploaded')).to_have_text('old session upload')
                        async with page.expect_download() as download:
                            await page.locator('#download').click()
                        assert '29' in Path(await (await download.value).path()).read_text(encoding='utf-8')
                        checks += ['CLI-reload', 'changed-worker-generation', 'live-session-and-transfer-continuity']
                        for invalid in (original + 'unknown_option=true\n', original.replace(str(public), str(free_port()), 1)):
                            config.write_text(invalid, encoding='utf-8')
                            await reload(400)
                            async with client.get(base + '/added/') as response:
                                assert response.status == 200
                        checks.append('invalid-and-listener-change-reloads-retain-working-configuration')
                        # A pre-existing multiplex transport must consult the new
                        # routing snapshot before it opens another application.
                        wire = Wire(client, base + '/fixture/__sockjs__')
                        await wire.connect()
                        config.write_text(original, encoding='utf-8')
                        await reload()
                        await wire.send('0|o|../added/')
                        assert (await wire.next())[:2] == ('0', 'c')
                        await wire.ws.close()
                        async with client.get(base + '/added/') as response:
                            assert response.status == 404
                        checks.append('removed-route-denied-on-existing-multiplex-transport')
                        if os.name != 'nt':
                            config.write_text(original + extra, encoding='utf-8')
                            process.send_signal(signal.SIGHUP)
                            async with asyncio.timeout(10):
                                while True:
                                    async with client.get(base + '/added/') as response:
                                        if response.status == 200:
                                            break
                                    await asyncio.sleep(.1)
                            checks.append('SIGHUP-reload')
                    finally:
                        await browser.close()
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'reload.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(exercise(parser.parse_args().binary))
