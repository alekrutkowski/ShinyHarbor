#!/usr/bin/env python3
"""Compare the same browser fixture against two running servers.

Both URLs must point to examples/compatibility deployed separately. These five
checks are smoke checks, NOT the complete required compatibility manifest.
"""
import argparse
import asyncio
import json
import sys
from pathlib import Path
from playwright.async_api import async_playwright
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from shiny_browser import exercise

async def compare(reference: str, candidate: str) -> dict:
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        results = []
        for url in [reference, candidate]:
            context = await browser.new_context()
            results.append(await exercise(await context.new_page(), url))
            await context.close()
        await browser.close()
    if results[0] != results[1]:
        raise RuntimeError(f"Behavior differs: {results}")
    return {"suite": "differential-smoke-only", "reference": reference, "candidate": candidate,
            "equal": True, "checks": results[0], "certifies_full_compatibility": False}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", required=True)
    parser.add_argument("--candidate", required=True)
    args = parser.parse_args()
    print(json.dumps(asyncio.run(compare(args.reference, args.candidate)), indent=2))
