"""Unicode/space app names, escaped mounts and safe directory listings."""
import argparse
import asyncio
import json
import re
import shutil
from test_support import TemporaryDirectory
from pathlib import Path
from urllib.parse import quote
from aiohttp import ClientSession
from yarl import URL
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]


async def main(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-paths-') as temp:
        root = Path(temp)
        site = root / 'site'
        site.mkdir()
        names = ['sales report', 'Zażółć 日本語', 'a & b']
        for name in names:
            directory = site / name
            directory.mkdir()
            shutil.copyfile(ROOT / 'examples/compatibility/app.R', directory / 'app.R')
        (site / 'a & b.txt').write_text('static content', encoding='utf-8')
        public, admin = free_port(), free_port()
        config = root / 'harbor.toml'
        config.write_text(f'''listen='127.0.0.1:{public}'
admin_listen='127.0.0.1:{admin}'
shutdown_grace_seconds=1
[[sites]]
name='files'
mount='/apps'
path='site'
directory_index=true
[[apps]]
name='explicit'
mount='/Raport 日本語'
path={json.dumps(str(site / names[0]))}
''', encoding='utf-8')
        with (results / 'unicode-paths.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                base = f'http://127.0.0.1:{public}'
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    try:
                        index = await browser.new_page()
                        await index.goto(base + '/apps/')
                        for name in names:
                            await expect(index.get_by_role('link', name=name + '/', exact=True)).to_have_attribute('href', quote(name, safe='') + '/')
                        checks.append('directory-labels-and-encoded-links')
                        for route in [*('/apps/' + quote(n, safe='') for n in names), '/' + quote('Raport 日本語', safe='')]:
                            page = await browser.new_page()
                            await page.goto(base + route + '/')
                            await expect(page.locator('#double')).to_have_text('4', timeout=30000)
                            await page.locator('#n').fill('17')
                            await page.locator('#n').dispatch_event('change')
                            await expect(page.locator('#double')).to_have_text('34')
                            await page.locator('#upload').set_input_files({'name':'value.txt', 'mimeType':'text/plain', 'buffer': b'path upload'})
                            await expect(page.locator('#uploaded')).to_have_text('path upload')
                            async with page.expect_download() as download:
                                await page.locator('#download').click()
                            assert '17' in Path(await (await download.value).path()).read_text(encoding='utf-8')
                            checks.append(route + ':reactivity-upload-download')
                            await page.close()
                    finally:
                        await browser.close()
                async with ClientSession() as client:
                    lower = re.sub(r'%[0-9A-F]{2}', lambda m: m.group().lower(), quote(names[1], safe=''))
                    async with client.get(URL(base + '/apps/' + lower + '/', encoded=True)) as response:
                        assert response.status == 200
                    async with client.get(base + '/apps/a%20%26%20b.txt') as response:
                        assert await response.text() == 'static content'
                    for unsafe in ('%2e%2e', '%2f', '%5c', '%00'):
                        async with client.get(URL(base + '/apps/' + unsafe + '/app.R', encoded=True)) as response:
                            assert response.status in (400, 404)
                    checks.append('equivalent-escaping-static-paths-and-traversal-denial')
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'unicode-paths.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
