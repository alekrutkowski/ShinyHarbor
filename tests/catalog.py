"""Check app discovery, safe links, branding and responsive catalog in a browser."""
import argparse
import asyncio
import json
import os
from pathlib import Path
from urllib.parse import quote
from aiohttp import ClientSession
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory

ROOT=Path(__file__).resolve().parents[1]

async def run(binary):
    results=ROOT/'test-results'; results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-catalog-') as tmp:
        root=Path(tmp); apps=root/'apps'; apps.mkdir()
        for name in ['hello', '日本語 & reports', '.hidden']:
            directory=apps/name; directory.mkdir()
            (directory/'app.R').write_text('library(shiny)\nshinyApp(fluidPage(), function(input, output) {})')
        (apps/'notes.txt').write_text('public notes')
        (apps/'secrets.R').write_text('source must not appear')
        public,admin=free_port(),free_port()
        config=root/'harbor.toml'
        config.write_text(f'listen="127.0.0.1:{public}"\nadmin_listen="127.0.0.1:{admin}"\nstate_dir="apps/private-state"\n[[sites]]\nname="catalog"\nmount="/"\npath="apps"\ndirectory_index=true\n')
        with (results/'catalog.log').open('wb') as log:
            proc=await asyncio.create_subprocess_exec(str(binary), 'serve','--config',str(config),stdout=log,stderr=log)
            try:
                await wait_ready(admin,proc)
                base=f'http://127.0.0.1:{public}'
                async with ClientSession() as client:
                    async with client.get(base+'/') as response:
                        text=await response.text()
                        assert response.status==200 and 'private-state' not in text and '.hidden' not in text and 'secrets.R' not in text
                    async with client.head(base+'/') as response:
                        assert response.status==200 and not await response.read()
                    async with client.get(base+'/__shinyharbor__/icon.svg') as response:
                        assert response.status==200 and '<svg' in await response.text()
                async with async_playwright() as p:
                    browser=await p.chromium.launch(channel=os.environ.get('SHINYHARBOR_TEST_BROWSER'))
                    try:
                        page=await browser.new_page(viewport={'width':1440,'height':1000})
                        await page.goto(base+'/')
                        await expect(page.get_by_role('heading',name='Your applications')).to_be_visible()
                        await expect(page.locator('.app-card')).to_have_count(2)
                        link=page.locator('.app-card').filter(has_text='日本語 & reports')
                        assert await link.get_attribute('href')==quote('日本語 & reports',safe='')+'/'
                        await page.screenshot(path=str(results/'catalog-desktop.png'),full_page=True)
                        await page.get_by_text('Folders and files',exact=True).click()
                        await expect(page.get_by_role('link',name='notes.txt')).to_be_visible()
                        await page.set_viewport_size({'width':390,'height':844})
                        assert await page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1')
                        await page.screenshot(path=str(results/'catalog-mobile.png'),full_page=True)
                    finally: await browser.close()
            finally:
                if proc.returncode is None: proc.terminate()
                await asyncio.wait_for(proc.wait(),15)
    report={'suite':'catalog','passed':True,'checks':['discovered-app-cards','encoded-Unicode-and-escaped-labels','private-state-and-R-source-hidden','static-files-accessible','HEAD-and-icon','desktop-and-mobile-layout']}
    (results/'catalog.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--binary',type=Path,required=True)
    asyncio.run(run(parser.parse_args().binary.resolve()))
