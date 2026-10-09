"""Real browser acceptance of the base-R statistical example and its downloads."""
import argparse
import asyncio
import csv
import io
import json
import os
from pathlib import Path
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]

async def run(binary, library):
    checks = []
    results = ROOT/'test-results'; results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-sampling-') as tmp:
        root=Path(tmp); public,admin=free_port(),free_port()
        while public == admin: admin=free_port()
        text=f'listen="127.0.0.1:{public}"\nadmin_listen="127.0.0.1:{admin}"\nshutdown_grace_seconds=1\n'
        if library: text += 'r_library='+json.dumps(str(library.resolve()))+'\n'
        text += '[[apps]]\nname="sampling"\nmount="/hello"\npath='+json.dumps(str(ROOT/'examples/hello'))+'\n'
        config=root/'harbor.toml'; config.write_text(text)
        with (results/'sampling-lab.log').open('wb') as log:
            proc=await asyncio.create_subprocess_exec(str(binary),'serve','--config',str(config),stdout=log,stderr=log)
            try:
                await wait_ready(admin,proc)
                async with async_playwright() as p:
                    browser=await p.chromium.launch(headless=True, channel=os.environ.get('SHINYHARBOR_TEST_BROWSER'))
                    try:
                        context=await browser.new_context(viewport={'width':1440,'height':1100})
                        page=await context.new_page(); errors=[]
                        page.on('pageerror',lambda e: errors.append(str(e)))
                        await page.goto(f'http://127.0.0.1:{public}/hello/')
                        await expect(page.locator('#scenario')).to_contain_text('2,000 repeated samples',timeout=60000)
                        await expect(page.locator('#metrics')).to_contain_text('Simulated coverage')
                        await expect(page.locator('#connection')).to_have_text('Live connection')
                        await expect(page.locator('#sample_plot img')).to_be_visible()
                        await page.screenshot(path=str(results/'sampling-lab-desktop.png'),full_page=True)
                        async def download(id):
                            async with page.expect_download() as pending: await page.locator('#'+id).click()
                            result=await pending.value
                            return Path(await result.path()).read_bytes()
                        sample=await download('sample_csv')
                        observations=list(csv.DictReader(io.StringIO(sample.decode())))
                        assert len(observations)==30 and all(row['seed']=='2026' for row in observations)
                        intervals=await download('intervals_csv')
                        draws=list(csv.DictReader(io.StringIO(intervals.decode())))
                        assert len(draws)==2000
                        for row in draws:
                            assert (float(row['lower']) <= float(row['true_mean']) <= float(row['upper'])) == (row['contains_true_mean']=='TRUE')
                        report=await download('report_pdf')
                        assert report.startswith(b'%PDF') and b'%%EOF' in report
                        checks += ['sample-and-interval-CSV-match-experiment','base-R-PDF-report','live-connection-and-plot']
                        other=await browser.new_context(); second=await other.new_page()
                        await second.goto(f'http://127.0.0.1:{public}/hello/')
                        await expect(second.locator('#scenario')).to_contain_text('Bell-shaped',timeout=60000)
                        # Native select has a selectize presentation; update through Shiny's input binding.
                        await page.evaluate("$('#shape')[0].selectize.setValue('skewed')")
                        await expect(page.locator('#pending')).to_contain_text('Settings changed')
                        await expect(page.locator('#scenario')).to_contain_text('Bell-shaped')
                        await page.locator('#run').click()
                        await expect(page.locator('#scenario')).to_contain_text('Right-skewed')
                        await expect(second.locator('#scenario')).to_contain_text('Bell-shaped')
                        await page.get_by_role('tab',name='Interval coverage',exact=True).click()
                        await expect(page.locator('#intervals_plot img')).to_be_visible()
                        await page.screenshot(path=str(results/'sampling-lab-coverage.png'),full_page=True)
                        await page.get_by_role('tab',name='Sampling distribution',exact=True).click()
                        await expect(page.locator('#means_plot img')).to_be_visible()
                        await page.set_viewport_size({'width':390,'height':844})
                        try:
                            await page.wait_for_function('document.documentElement.scrollWidth <= innerWidth + 1', timeout=5000)
                        except Exception:
                            print(await page.evaluate("[...document.querySelectorAll('body *')].filter(n=>n.getBoundingClientRect().right>innerWidth+1).slice(0,12).map(n=>({tag:n.tagName,id:n.id,class:n.className,right:n.getBoundingClientRect().right}))"))
                            await page.screenshot(path=str(results/'sampling-lab-mobile-failure.png'),full_page=True)
                            raise
                        await page.screenshot(path=str(results/'sampling-lab-mobile.png'),full_page=True)
                        assert not errors,errors
                        checks += ['explicit-run-keeps-experiments-consistent','independent-browser-session-results','all-three-statistical-plots','responsive-mobile-layout','no-browser-script-errors']
                    finally: await browser.close()
            finally:
                if proc.returncode is None: proc.terminate()
                await asyncio.wait_for(proc.wait(),15)
    report={'suite':'sampling-lab-browser','passed':True,'checks':checks}
    (results/'sampling-lab.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--binary',type=Path,required=True); parser.add_argument('--r-library',type=Path)
    args=parser.parse_args(); asyncio.run(run(args.binary.resolve(),args.r_library))
