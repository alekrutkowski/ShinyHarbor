#!/usr/bin/env python3
"""Real Shiny smoke checks. This is deliberately NOT a full conformance suite."""
from __future__ import annotations
import argparse
import asyncio
import json
import os
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
ROOT = Path(__file__).resolve().parents[1]


async def exercise(page, url: str) -> dict:
    await page.goto(url, wait_until="domcontentloaded")
    await expect(page.locator("#double")).to_have_text("4", timeout=60000)
    await page.locator("#n").fill("7")
    await page.locator("#n").dispatch_event("change")
    await expect(page.locator("#double")).to_have_text("14")
    await page.locator("#text").fill("Zażółć 日本語 ✓")
    await page.locator("#text").dispatch_event("change")
    await expect(page.locator("#echo")).to_have_text("Zażółć 日本語 ✓")
    await page.locator("#upload").set_input_files({"name": "test.txt", "mimeType": "text/plain", "buffer": b"uploaded-through-rust"})
    await expect(page.locator("#uploaded")).to_have_text("uploaded-through-rust")
    await page.locator("#dynamic").click()
    await expect(page.locator("#added")).to_have_text("Dynamic UI is working")
    async with page.expect_download() as info:
        await page.locator("#download").click()
    download = await info.value
    assert download.suggested_filename == "hello world.csv", download.suggested_filename
    path = await download.path()
    content = Path(path).read_text(encoding="utf-8")
    assert "Zażółć 日本語 ✓" in content and "7" in content
    return {"reactivity": True, "unicode": True, "upload": True, "download": True, "dynamic-ui": True}


async def main(binary: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="harbor-shiny-") as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        while admin == public:
            admin = free_port()
        config = root / "harbor.toml"
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
            "[[apps]]\nname='fixture'\nmount='/nested/fixture'\n"
            f"path={json.dumps(str(ROOT / 'examples/compatibility'))}\nstartup_seconds=90\n")
        results = ROOT / "test-results"
        results.mkdir(exist_ok=True)
        with (results / "shiny-server.log").open("wb") as log:
            proc = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, proc)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True, args=["--no-sandbox"])
                    page = await browser.new_page()
                    result = await exercise(page, f"http://127.0.0.1:{public}/nested/fixture/")
                    await page.screenshot(path=str(results / "real-shiny.png"), full_page=True)
                    await browser.close()
            finally:
                if proc.returncode is None:
                    proc.terminate()
                    try:
                        await asyncio.wait_for(proc.wait(), 10)
                    except asyncio.TimeoutError:
                        proc.kill()
                        await proc.wait()
        report = {"suite": "real-shiny-smoke", "checks": result, "passed": True, "certifies_full_compatibility": False}
        (results / "real-shiny.json").write_text(json.dumps(report, indent=2))
        return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
