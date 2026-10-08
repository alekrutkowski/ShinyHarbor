"""Real browser acceptance cases for native sites, restarts and bookmarks.

Requires the compiled Rust executable, R/Shiny and Playwright. These cases extend
smoke coverage; they are not a declaration of complete upstream conformance.
"""
from __future__ import annotations
import argparse
import asyncio
import json
import shutil
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]


async def exercise(binary: Path):
    checks = []
    results = ROOT / "test-results"
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="harbor-native-") as temp:
        root = Path(temp)
        public = free_port()
        # Legacy config uses the default private admin port, so derive public
        # independently. Platform jobs execute this suite serially.
        while public == 3839:
            public = free_port()
        site = root / "site"
        site.mkdir()
        (site / "data.txt").write_text("0123456789", encoding="utf-8")
        (site / ".private").write_text("private", encoding="utf-8")
        (site / "source.R").write_text("private R source", encoding="utf-8")
        config = root / "shiny-server.conf"
        config.write_text(f"""
# Deliberately use original directive syntax, not TOML.
run_as :PROCESS_USER:;
app_idle_timeout 0;
server {{
  listen {public} 127.0.0.1;
  location /reports {{
    site_dir "{site.as_posix()}";
    directory_index on;
    frame_options sameorigin;
  }}
  location /old {{ redirect /reports 302 false; }}
}}
""", encoding="utf-8")
        with (results / "native-compat.log").open("wb") as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, process)
                base = f"http://127.0.0.1:{public}"
                async with ClientSession() as client:
                    async with client.get(base + "/reports/data.txt", headers={"Range": "bytes=2-5"}) as response:
                        assert response.status == 206
                        assert await response.text() == "2345"
                    async with client.head(base + "/reports/data.txt") as response:
                        assert response.status == 200 and response.headers["Content-Length"] == "10"
                        assert await response.read() == b""
                        modified = response.headers["Last-Modified"]
                    async with client.get(base + "/reports/data.txt", headers={"If-Modified-Since": modified}) as response:
                        assert response.status == 304
                    for path in [".private", "source.R", "missing.txt"]:
                        async with client.get(base + "/reports/" + path) as response:
                            assert response.status == 404, (path, response.status)
                    async with client.get(base + "/old/data.txt?x=1&x=2", allow_redirects=False) as response:
                        assert response.status == 302
                        assert response.headers["Location"] == "/reports/data.txt?x=1&x=2"
                checks += ["legacy-configuration", "static-range", "static-head", "static-cache-validator", "source-and-hidden-files-denied", "redirect-query"]
                # Add the app after the server is already running, two levels down.
                app = site / "nested" / "session"
                shutil.copytree(ROOT / "examples/session", app)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    old_context = await browser.new_context()
                    page = await old_context.new_page()
                    response = await page.goto(base + "/reports/nested/session/", wait_until="domcontentloaded")
                    assert response.headers.get("x-frame-options") == "SAMEORIGIN"
                    await expect(page.locator("#double")).to_have_text("4", timeout=60000)
                    await expect(page.locator("#identity")).to_have_text("anonymous")
                    old_pid = await page.locator("#pid").inner_text()
                    await page.locator("#n").fill("13")
                    await page.locator("#n").dispatch_event("change")
                    await expect(page.locator("#double")).to_have_text("26")
                    await page.locator("#save").click()
                    await expect(page.locator("#bookmark_url")).to_contain_text("_state_id_=")
                    bookmark = await page.locator("#bookmark_url").inner_text()
                    checks += ["dynamic-nested-discovery", "frame-options", "anonymous-identity", "server-bookmark-save"]
                    (app / "restart.txt").touch()
                    new_context = await browser.new_context()
                    restored = await new_context.new_page()
                    await restored.goto(bookmark, wait_until="domcontentloaded")
                    await expect(restored.locator("#double")).to_have_text("26", timeout=60000)
                    await expect(restored.locator("#pid")).not_to_have_text(old_pid)
                    await page.locator("#n").fill("17")
                    await page.locator("#n").dispatch_event("change")
                    await expect(page.locator("#double")).to_have_text("34")
                    await expect(page.locator("#pid")).to_have_text(old_pid)
                    checks += ["new-worker-after-restart", "bookmark-restored-across-workers", "existing-session-survives-restart"]
                    await restored.screenshot(path=str(results / "native-bookmark.png"))
                    await browser.close()
            finally:
                if process.returncode is None:
                    process.terminate()
                    try:
                        await asyncio.wait_for(process.wait(), 15)
                    except asyncio.TimeoutError:
                        process.kill(); await process.wait()
    report = {"suite": "native-compatibility-cases", "passed": True, "checks": checks, "certifies_full_compatibility": False}
    (results / "native-compat.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    print(json.dumps(asyncio.run(exercise(parser.parse_args().binary)), indent=2))
