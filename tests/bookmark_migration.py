"""Read real server-side bookmarks in both directions between two R hosts.

Both URLs must host examples/session with the same app path, OS account and
bookmark_state_dir. Use an isolated test store, not production bookmarks.
"""
import argparse
import asyncio
import json
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

from playwright.async_api import async_playwright, expect


async def main(reference, candidate):
    checks = []
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=True)
        try:
            for source, target, value, label in [
                (reference, candidate, 37, 'upstream-bookmark-restored-by-ShinyHarbor'),
                (candidate, reference, 41, 'ShinyHarbor-bookmark-restored-by-upstream'),
            ]:
                context = await browser.new_context()
                page = await context.new_page()
                await page.goto(source)
                await expect(page.locator('#double')).to_have_text('4', timeout=60000)
                await page.locator('#n').fill(str(value))
                await page.locator('#n').dispatch_event('change')
                await expect(page.locator('#double')).to_have_text(str(value * 2))
                await page.locator('#save').click()
                await expect(page.locator('#bookmark_url')).to_contain_text('_state_id_=')
                saved = urlsplit(await page.locator('#bookmark_url').inner_text())
                destination = urlsplit(target)
                restored_url = urlunsplit((destination.scheme, destination.netloc,
                                          destination.path, saved.query, saved.fragment))
                # No affinity cookies or source-session state cross into the target.
                other = await browser.new_context()
                restored = await other.new_page()
                await restored.goto(restored_url)
                await expect(restored.locator('#double')).to_have_text(str(value * 2), timeout=60000)
                await expect(restored.locator('#n')).to_have_value(str(value))
                checks.append(label)
                await other.close()
                await context.close()
        finally:
            await browser.close()
    report = {'suite': 'bookmark-migration', 'passed': True,
              'reference': reference, 'candidate': candidate, 'checks': checks,
              'certifies_full_compatibility': False}
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    (results / 'bookmark-migration.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--reference', required=True)
    parser.add_argument('--candidate', required=True)
    args = parser.parse_args()
    asyncio.run(main(args.reference, args.candidate))
