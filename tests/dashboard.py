#!/usr/bin/env python3
"""Browser test of the actual embedded dashboard, with explicitly synthetic API data.

This verifies the UI, not Rust compilation or R/Shiny compatibility.
"""
from __future__ import annotations
import asyncio
import json
import os
import re
import time
from pathlib import Path
from aiohttp import web
from playwright.async_api import async_playwright
ROOT = Path(__file__).resolve().parents[1]
CSP = "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
TOKEN = "synthetic-dashboard-test-token-not-a-real-secret"


def fixture() -> dict:
    now = int(time.time())
    return {"days": 7, "generated_at": now, "enabled": True, "country_database_loaded": True,
        "dropped_events": 0,
        "apps": [
            {"app": "sales-dashboard", "requests": 38620, "pages": 8240, "visitor_days": 2431, "websockets": 7932, "errors": 3, "bytes_in": 9172380, "bytes_out": 725090321},
            {"app": "forecast-studio", "requests": 21440, "pages": 3612, "visitor_days": 982, "websockets": 3452, "errors": 0, "bytes_in": 3000941, "bytes_out": 193021271},
            {"app": "research-explorer", "requests": 8241, "pages": 1440, "visitor_days": 453, "websockets": 1341, "errors": 1, "bytes_in": 420941, "bytes_out": 76432127}],
        "pools": [
            {"name": "sales-dashboard", "mount": "/sales", "runtime": "native-r", "active_websockets": 18, "workers": 3},
            {"name": "forecast-studio", "mount": "/forecast", "runtime": "native-r", "active_websockets": 7, "workers": 2},
            {"name": "research-explorer", "mount": "/research", "runtime": "upstream", "active_websockets": 4, "workers": 1}],
        "series": [{"hour": now // 3600 - 168 + i, "pages": int(24 + (i % 24) * 2 + (i * 37 % 29) + (i // 24) * 4), "requests": 100} for i in range(169)],
        "countries": [{"code": "NL", "pages": 4821}, {"code": "US", "pages": 3604}, {"code": "DE", "pages": 2241}, {"code": "GB", "pages": 1643}, {"code": "ZZ", "pages": 983}],
        "referrers": [{"origin": "", "pages": 7140}, {"origin": "https://intranet.example.org", "pages": 4201}, {"origin": "https://research.example.org", "pages": 1314}, {"origin": "https://docs.example.org", "pages": 637}]}


async def run() -> dict:
    async def serve(request):
        if request.path == "/api/stats":
            if request.headers.get("Authorization") != "Bearer " + TOKEN:
                return web.Response(status=401)
            data = fixture()
            data["days"] = int(request.query.get("days", 7))
            return web.json_response(data)
        names = {"/": "index.html", "/style.css": "style.css", "/app.js": "app.js"}
        name = names.get(request.path)
        if not name:
            return web.Response(status=404)
        response = web.FileResponse(ROOT / "assets" / name)
        response.headers["Content-Security-Policy"] = CSP
        return response
    app = web.Application()
    app.router.add_route("GET", "/{path:.*}", serve)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    base = f"http://127.0.0.1:{site._server.sockets[0].getsockname()[1]}"
    checks = []
    in_memory = os.environ.get("HARBOR_UI_IN_MEMORY") == "1"
    results = ROOT / "test-results"
    results.mkdir(exist_ok=True)
    try:
        async with async_playwright() as p:
            executable = os.environ.get("CHROMIUM_EXECUTABLE")
            browser = await p.chromium.launch(executable_path=executable, headless=True, args=["--no-sandbox"])
            page = await browser.new_page(viewport={"width": 1512, "height": 1200}, color_scheme="light")
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            if in_memory:
                # Render our own assets without network navigation. This does not
                # exercise the server's HTTP headers or the browser's CSP handling.
                html = (ROOT / "assets/index.html").read_text(encoding="utf-8")
                html = re.sub(r'<link[^>]+>|<script[^>]*>.*?</script>', '', html)
                await page.set_content(html)
                await page.add_style_tag(content=(ROOT / "assets/style.css").read_text(encoding="utf-8"))
                await page.evaluate("""({token, fixture}) => {
                    window.fetch = async (url, options) => {
                        if (options.headers.Authorization !== 'Bearer ' + token)
                            return new Response('', {status: 401});
                        const data = structuredClone(fixture);
                        data.days = Number(new URL(url, 'https://test.invalid').searchParams.get('days'));
                        return new Response(JSON.stringify(data), {status: 200, headers: {'Content-Type': 'application/json'}});
                    };
                }""", {"token": TOKEN, "fixture": fixture()})
                await page.add_script_tag(content=(ROOT / "assets/app.js").read_text(encoding="utf-8"))
            else:
                await page.goto(base)
            assert await page.locator("#login").is_visible()
            assert not await page.locator("#content").is_visible()
            checks.append("locked-by-default")
            await page.locator("#token").fill("invalid-token")
            await page.locator("#login-form button").click()
            await page.wait_for_function("document.querySelector('#message').textContent.includes('not accepted')")
            checks.append("rejects-invalid-token")
            await page.locator("#token").fill(TOKEN)
            await page.locator("#login-form button").click()
            await page.locator("#content").wait_for(state="visible")
            assert await page.locator("#pages").inner_text() == "13,292"
            assert await page.locator("#active").inner_text() == "29"
            assert await page.locator("#app-rows tr").count() == 3
            checks += ["renders-aggregates", "renders-runtime", "renders-applications"]
            if not in_memory:
                assert await page.evaluate("localStorage.length === 0 && sessionStorage.length === 0")
            assert await page.locator("#token").input_value() == ""
            checks.append("token-cleared-from-input" if in_memory else "no-persistent-token-storage")
            await page.locator("header .eyebrow").evaluate("e => e.textContent = 'SYNTHETIC PREVIEW · NOT LIVE DATA'")
            await page.screenshot(path=str(results / "dashboard-synthetic-desktop.png"), full_page=True)
            await page.locator("#sort-pages").click()
            assert "research-explorer" in await page.locator("#app-rows tr").first.inner_text()
            checks.append("table-sort")
            await page.locator("#days").select_option("1")
            await page.wait_for_function("document.querySelector('#chart').getAttribute('aria-label').includes('1 day(s)')")
            checks.append("period-switch")
            await page.set_viewport_size({"width": 390, "height": 844})
            assert await page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            await page.screenshot(path=str(results / "dashboard-synthetic-mobile.png"), full_page=True)
            checks.append("mobile-no-horizontal-overflow")
            await page.set_viewport_size({"width": 1512, "height": 1200})
            await page.locator("#lock").click()
            assert await page.locator("#login").is_visible()
            assert not await page.locator("#content").is_visible()
            checks.append("explicit-lock")
            assert not errors, errors
            checks.append("no-browser-javascript-errors")
            await browser.close()
    finally:
        await runner.cleanup()
    result = {"suite": "dashboard-browser-synthetic-api", "mode": "in-memory-assets" if in_memory else "http-with-csp", "server_http_or_csp_tested": not in_memory, "passed": True, "checks": checks, "count": len(checks)}
    (results / "dashboard.json").write_text(json.dumps(result, indent=2))
    return result


if __name__ == "__main__":
    print(json.dumps(asyncio.run(run()), indent=2))
