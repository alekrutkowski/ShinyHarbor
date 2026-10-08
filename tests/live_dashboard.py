"""Exercise the shipped dashboard against real R traffic and the real SQLite API."""
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from shiny_browser import exercise

ROOT = Path(__file__).resolve().parents[1]

async def main(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-live-ui-') as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        config = root / 'harbor.toml'
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\n"
            "state_dir='state'\nshutdown_grace_seconds=1\n[[apps]]\nname='live'\nlabel='Live Shiny test'\nmount='/live'\n"
            f"path={json.dumps(str(ROOT / 'examples/compatibility'))}\n", encoding='utf-8')
        with (results / 'live-dashboard.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                token = (root / 'state/admin.token').read_text().strip()
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    page = await browser.new_page()
                    await exercise(page, f'http://127.0.0.1:{public}/live/')
                    dashboard = await browser.new_page(viewport={'width':1440, 'height':1100})
                    errors = []
                    dashboard.on('pageerror', lambda error: errors.append(str(error)))
                    await dashboard.goto(f'http://127.0.0.1:{admin}/')
                    await dashboard.locator('#token').fill(token)
                    await dashboard.locator('#login-form button').click()
                    await expect(dashboard.locator('#content')).to_be_visible()
                    await expect(dashboard.locator('#app-rows')).to_contain_text('Live Shiny test')
                    await expect(dashboard.locator('#pages')).not_to_have_text('0')
                    await expect(dashboard.locator('#active')).not_to_have_text('0')
                    assert not errors, errors
                    await dashboard.screenshot(path=str(results / 'live-dashboard.png'), full_page=True)
                    await dashboard.locator('#lock').click()
                    await expect(dashboard.locator('#login')).to_be_visible()
                    assert not await dashboard.evaluate('localStorage.length || sessionStorage.length')
                    await browser.close()
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 15)
    report = {'suite':'dashboard-real-R-and-SQLite', 'passed':True,
              'checks':['real-page-count', 'real-live-session-count', 'application-label', 'authenticated-dashboard', 'lock-clears-access', 'no-persistent-browser-token']}
    (results / 'live-dashboard.json').write_text(json.dumps(report, indent=2))
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
