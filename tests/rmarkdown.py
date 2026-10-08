"""Interactive R Markdown path and resource integration through real R/Pandoc."""
from __future__ import annotations
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]
DOCUMENT = '''---
title: "Interactive document"
output: html_document
runtime: shiny
---

```{r, echo=FALSE}
shiny::numericInput("value", "Value", 3)
shiny::textOutput("answer")
output$answer <- shiny::renderText(paste("Result:", input$value * 7))
shiny::tags$img(src="pixel.svg", id="local-asset")
```
'''


async def main(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with tempfile.TemporaryDirectory(prefix='harbor-rmarkdown-') as temp:
        root = Path(temp)
        site = root / 'site'
        index = site / 'index'
        index.mkdir(parents=True)
        (index / 'index.Rmd').write_text(DOCUMENT, encoding='utf-8')
        (site / 'report.Rmd').write_text(DOCUMENT, encoding='utf-8')
        for directory in (site, index):
            (directory / 'pixel.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="blue"/></svg>', encoding='utf-8')
        public, admin = free_port(), free_port()
        config = root / 'harbor.toml'
        config.write_text(f'''listen='127.0.0.1:{public}'
admin_listen='127.0.0.1:{admin}'
shutdown_grace_seconds=1
[[sites]]
name='documents'
mount='/docs'
path={json.dumps(str(site))}
[[apps]]
name='explicit'
mount='/direct'
path={json.dumps(str(site / 'report.Rmd'))}
''', encoding='utf-8')
        with (results / 'rmarkdown.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    try:
                        for route in ('/docs/index/', '/docs/report.Rmd/', '/direct/'):
                            page = await browser.new_page()
                            failed = []
                            page.on('response', lambda response: failed.append(response.url) if response.status >= 400 else None)
                            await page.goto(f'http://127.0.0.1:{public}' + route, wait_until='domcontentloaded')
                            await expect(page.locator('#answer')).to_have_text('Result: 21', timeout=60000)
                            await page.locator('#value').fill('11')
                            await page.locator('#value').dispatch_event('change')
                            await expect(page.locator('#answer')).to_have_text('Result: 77')
                            assert await page.locator('#local-asset').evaluate('(img) => img.complete && img.naturalWidth === 8')
                            assert not failed, failed
                            checks.append(route + ':reactive-document-and-relative-resources')
                            await page.close()
                    finally:
                        await browser.close()
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'rmarkdown.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    asyncio.run(main(parser.parse_args().binary))
