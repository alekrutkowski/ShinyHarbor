"""Admin authorization, transactional editing, browser UX, and terminal snapshots."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import sys
import tomllib
from aiohttp import ClientSession
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]

async def run(binary, skip_browser=False):
    checks = []
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-admin-') as tmp:
        root = Path(tmp)
        public, admin = free_port(), free_port()
        while public == admin:
            admin = free_port()
        (root / 'index.html').write_text('public site')
        config = root / 'harbor.toml'
        original = (f'# retain this comment\nlisten="127.0.0.1:{public}"\nadmin_listen="127.0.0.1:{admin}"\n'
                    'state_dir="state"\nshutdown_grace_seconds=1\n'
                    '[[sites]]\nname="site"\nmount="/"\npath="."\ndirectory_index=true\n')
        config.write_text(original, encoding='utf-8', newline='\n')
        with (results / 'management.log').open('wb') as log:
            proc = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, proc)
                token = (root / 'state/admin.token').read_text().strip()
                headers = {'Authorization':'Bearer ' + token}
                base = f'http://127.0.0.1:{admin}'
                async with ClientSession() as client:
                    for method in [client.get, client.post]:
                        kwargs = {} if method == client.get else {'json':{'revision':'x','source':original,'save':True}}
                        async with method(base+'/api/config', **kwargs) as r:
                            assert r.status == 401, r.status
                    async with client.get(f'http://127.0.0.1:{public}/api/config', headers=headers) as r:
                        assert r.status == 404
                    checks += ['unauthenticated-read-and-write-denied','configuration-absent-from-app-listener']
                    async def get():
                        async with client.get(base+'/api/config', headers=headers) as r:
                            assert r.status == 200
                            assert r.headers['Cache-Control'] == 'no-store'
                            return await r.json()
                    async def edit(body, status=200, extra=None):
                        async with client.post(base+'/api/config', headers={**headers, **(extra or {})}, json=body) as r:
                            result = await r.json()
                            assert r.status == status, result
                            return result
                    initial = await get()
                    assert initial['source'] == original and not initial['pending']
                    payload = {'revision':initial['revision'], 'changes':{'response_headers':{'X-Admin-Test':'applied'}}}
                    await edit(payload, 403, {'Origin':'https://unrelated.example'})
                    await edit({**payload,'changes':{'admin_listen':'0.0.0.0:4567'}}, 400)
                    await edit({**payload,'changes':{'tls':{'mode':'pem','cert':'absent.pem','key':'absent.key'}}}, 400)
                    assert config.read_text() == original
                    checks += ['cross-origin-edit-rejected','invalid-config-and-missing-certificate-do-not-write']
                    preview = await edit(payload)
                    assert preview['valid'] and not preview['saved'] and not preview['restart_required']
                    assert config.read_text() == original
                    saved = await edit({**payload,'save':True})
                    assert saved['saved'] and not saved['restart_required']
                    assert Path(saved['backup']).read_text() == original
                    assert '# retain this comment' in config.read_text()
                    async with client.get(f'http://127.0.0.1:{public}/') as r:
                        assert r.headers['X-Admin-Test'] == 'applied'
                    backup = Path(saved['backup'])
                    async with client.get(f'http://127.0.0.1:{public}/.shinyharbor-backups/{backup.name}') as r:
                        assert r.status == 404
                    await edit({**payload,'save':True}, 409)
                    checks += ['validate-without-write','atomic-save-and-comment-preservation','live-application','private-backup','stale-save-conflict']
                    current = await get()
                    next_port = free_port()
                    changed = await edit({'revision':current['revision'],'changes':{'listen':f'127.0.0.1:{next_port}'},'save':True})
                    assert changed['restart_required'] and changed['saved']
                    pending = await get()
                    assert pending['pending'] and pending['active']['listen'].endswith(str(public))
                    assert pending['configured']['listen'].endswith(str(next_port))
                    await edit({'revision':pending['revision'],'source':current['source'],'save':True})
                    checks.append('saved-versus-running-and-restart-required')
                    # External-editor conflict, in addition to a second browser's save.
                    stale = await get()
                    config.write_text(config.read_text() + '\n# external edit\n')
                    await edit({'revision':stale['revision'],'source':stale['source'],'save':True}, 409)
                    checks.append('external-file-edit-conflict')
                if not skip_browser:
                    from playwright.async_api import async_playwright, expect
                    async with async_playwright() as p:
                        browser = await p.chromium.launch(headless=True, channel=os.environ.get('SHINYHARBOR_TEST_BROWSER'))
                        try:
                            page = await browser.new_page(viewport={'width':1440,'height':1080})
                            await page.goto(base+'/admin')
                            await expect(page.locator('#content')).to_be_hidden()
                            await page.locator('#token').fill(token)
                            await page.locator('#unlock').click()
                            await expect(page.locator('#content')).to_be_visible()
                            await page.locator('#setting-analytics-retention_days').fill('45')
                            await page.locator('#tab-help').click()
                            await expect(page.locator('#reference')).to_contain_text('TLS')
                            await page.locator('#tab-guided').click()
                            await page.locator('#validate').click()
                            await expect(page.locator('#save')).to_be_enabled()
                            await expect(page.locator('#validation')).to_contain_text('restart')
                            await page.locator('#save').click()
                            await expect(page.locator('#message')).to_contain_text('Saved with a backup')
                            assert tomllib.loads(config.read_text())['analytics']['retention_days'] == 45
                            await page.evaluate('scrollTo(0,0)')
                            await page.screenshot(path=str(results/'admin-desktop.png'), full_page=True)
                            await page.locator('#tab-active').click()
                            await page.locator('#filter').fill('retention_days')
                            await expect(page.locator('#active-values')).to_contain_text('30')
                            await page.set_viewport_size({'width':390,'height':844})
                            assert await page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1')
                            await page.screenshot(path=str(results/'admin-mobile.png'), full_page=True)
                            await page.locator('#lock').click()
                            await expect(page.locator('#content')).to_be_hidden()
                            assert await page.evaluate('localStorage.length + sessionStorage.length') == 0
                            checks += ['browser-guided-edit-help-and-preview','browser-save-and-restart-explanation','responsive-admin-layout','lock-clears-token-without-browser-storage']
                        finally:
                            await browser.close()
                for args, expected in [(['open','admin','--print'], base+'/admin'), (['open','apps','--print'],f'http://127.0.0.1:{public}/')]:
                    child = await asyncio.create_subprocess_exec(str(binary),*args,'--config',str(config), stdout=asyncio.subprocess.PIPE)
                    stdout,_ = await child.communicate(); assert child.returncode == 0 and stdout.decode().strip() == expected
                child = await asyncio.create_subprocess_exec(str(binary),'tui','--once','--config',str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                stdout,stderr = await child.communicate()
                assert child.returncode == 0, stderr
                assert b'SHINYHARBOR' in stdout and b'CONNECTED' in stdout and b'/admin' in stdout
                (results/'tui.txt').write_bytes(stdout)
                checks += ['CLI-links-respect-configured-ports','TUI-live-status-snapshot']
            finally:
                if proc.returncode is None: proc.terminate()
                await asyncio.wait_for(proc.wait(), 15)
        # Legacy editing retains its syntax, includes and relative path resolution.
        legacy = root/'shiny-server.conf'
        legacy.write_text(f'server {{ listen {public} 127.0.0.1; location / {{ site_dir .; directory_index on; }} }}\n')
        # Legacy uses the default admin port, so do this after the isolated test.
        with (results/'management-legacy.log').open('wb') as log:
            proc = await asyncio.create_subprocess_exec(str(binary),'serve','--config',str(legacy), stdout=log,stderr=log)
            try:
                await wait_ready(3839,proc)
                token = (root/'.harbor/admin.token').read_text().strip()
                async with ClientSession(headers={'Authorization':'Bearer '+token}) as client:
                    async with client.get('http://127.0.0.1:3839/api/config') as r: data = await r.json()
                    assert data['format'] == 'legacy'
                    async with client.post('http://127.0.0.1:3839/api/config',json={'revision':data['revision'],'source':data['source']+'\n# retained legacy syntax\n','save':True}) as r:
                        assert r.status == 200, await r.text()
                    assert '# retained legacy syntax' in legacy.read_text()
                checks.append('legacy-conf-source-edit')
            finally:
                if proc.returncode is None: proc.terminate()
                await asyncio.wait_for(proc.wait(),15)
    report={'suite':'management','passed':True,'browser_tested':not skip_browser,'checks':checks}
    (results/'management.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report,indent=2))

if __name__ == '__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--skip-browser',action='store_true',help='Exercise API and CLI only; report browser coverage as untested')
    args=parser.parse_args(); asyncio.run(run(args.binary.resolve(),args.skip_browser))
