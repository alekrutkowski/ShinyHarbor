"""Exercise real Shiny over each supported SockJS browser transport."""
from __future__ import annotations
import argparse
import asyncio
import json
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession
from playwright.async_api import async_playwright
from gateway import free_port, wait_ready
from shiny_browser import exercise

ROOT = Path(__file__).resolve().parents[1]
PROTOCOLS = ["websocket", "xhr-streaming", "xdr-streaming", "eventsource",
             "iframe-eventsource", "htmlfile", "iframe-htmlfile", "xhr-polling",
             "xdr-polling", "iframe-xhr-polling", "jsonp-polling"]
# XDomainRequest is an obsolete IE-only API; modern Chromium cannot execute it.
MODES = ["websocket", "xhr-streaming", "eventsource", "htmlfile", "xhr-polling",
         "iframe-xhr-polling", "jsonp-polling"]

async def main(binary: Path):
    results = ROOT / "test-results"
    results.mkdir(exist_ok=True)
    checks = {}
    with TemporaryDirectory(prefix="harbor-transports-") as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        while admin == public:
            admin = free_port()
        config = root / "harbor.toml"
        text = f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\nsockjs_heartbeat_seconds=0.5\nsockjs_disconnect_seconds=1\n"
        for mode in MODES:
            # Upstream aliases treat disabling an iframe transport as also
            # disabling its direct equivalent, so leave that alias available.
            keep = {mode}
            if mode in {"eventsource", "htmlfile"}:
                keep.add("iframe-" + mode)
            text += (f"\n[[apps]]\nname='{mode}'\nmount='/{mode}'\n"
                     f"path={json.dumps(str(ROOT / 'examples/compatibility'))}\n"
                     f"disable_protocols={json.dumps([p for p in PROTOCOLS if p not in keep])}\n"
                     "max_workers=1\nidle_seconds=1\n")
        config.write_text(text, encoding="utf-8")
        with (results / "transports.log").open("wb") as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    try:
                        for mode in MODES:
                            context = await browser.new_context()
                            page = await context.new_page()
                            errors = []
                            page.on("pageerror", lambda e: errors.append(str(e)))
                            requests = []
                            page.on("response", lambda r: requests.append((r.status, r.url)))
                            try:
                                checks[mode] = await exercise(page, f"http://127.0.0.1:{public}/{mode}/")
                            except Exception:
                                print(json.dumps({"mode": mode, "errors": errors, "requests": requests}, indent=2), flush=True)
                                raise
                            assert not errors, (mode, errors)
                            await context.close()
                            print(f"Passed {mode}", flush=True)
                    finally:
                        await browser.close()
                async with ClientSession() as client:
                    base = f"http://127.0.0.1:{public}/xhr-polling/__sockjs__"
                    async with client.get(base + "/info") as response:
                        assert response.status == 200
                        assert not (await response.json())["websocket"]
                    async with client.get(base + "/info", headers={"Origin": "https://foreign.example"}) as response:
                        assert response.status == 403
                    async with client.post(base + "/000/not-created/xhr_send", data='["invalid"]') as response:
                        assert response.status == 404
                    async with client.get(base + "/000/test/jsonp?c=x%28%29") as response:
                        assert response.status in (404, 500)
                    checks["protocol-security"] = True
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 15)
    report = {"suite": "real-shiny-transports", "passed": True, "checks": checks,
              "certifies_full_compatibility": False}
    (results / "transports.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    return report

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
