"""Two tabs sharing cookies must keep independent sessions and downloads."""
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]

async def main(binary):
    with tempfile.TemporaryDirectory(prefix="harbor-tabs-") as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        conf = root / "harbor.toml"
        conf.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
                        "[[apps]]\nname='tabs'\nmount='/tabs'\nmax_workers=2\nsessions_per_worker=1\n"
                        f"path={json.dumps(str(ROOT / 'examples/compatibility'))}\n", encoding="utf-8")
        with (ROOT / "test-results/multiworker.log").open("wb") as log:
            proc = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(conf), stdout=log, stderr=log)
            try:
                await wait_ready(admin, proc)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    context = await browser.new_context()
                    pages = []
                    for number in (17, 29):
                        page = await context.new_page()
                        await page.goto(f"http://127.0.0.1:{public}/tabs/")
                        await expect(page.locator("#double")).to_have_text("4", timeout=30000)
                        await page.locator("#n").fill(str(number))
                        await page.locator("#n").dispatch_event("change")
                        await expect(page.locator("#double")).to_have_text(str(number * 2))
                        pages.append((page, number))
                    assert await pages[0][0].locator("#pid").inner_text() != await pages[1][0].locator("#pid").inner_text()
                    for page, number in pages:
                        async with page.expect_download() as info:
                            await page.locator("#download").click()
                        download = await info.value
                        assert str(number) in Path(await download.path()).read_text(encoding="utf-8")
                        await page.locator("#upload").set_input_files({"name":"tab.txt", "mimeType":"text/plain", "buffer":str(number).encode()})
                        await expect(page.locator("#uploaded")).to_have_text(str(number))
                    await browser.close()
            finally:
                proc.terminate()
                await asyncio.wait_for(proc.wait(), 15)
    report = {"suite":"multiworker-tabs", "passed":True, "checks":["shared-cookie-tabs", "distinct-worker-pids", "session-download-routing", "session-upload-routing"]}
    (ROOT / "test-results/multiworker.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    return report

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
