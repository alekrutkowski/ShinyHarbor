"""Render the repository SVG and package native icons. Requires Playwright/Chromium."""
import asyncio
import os
from pathlib import Path
import struct
from playwright.async_api import async_playwright

ROOT = Path(__file__).resolve().parents[1]

async def main():
    folder = ROOT / 'assets/brand'
    source = (folder / 'shinyharbor.svg').read_text(encoding='utf-8')
    frames = {}
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, channel=os.environ.get('SHINYHARBOR_TEST_BROWSER'))
        try:
            page = await browser.new_page(device_scale_factor=1)
            for size in [16, 24, 32, 48, 64, 128, 256, 512, 1024]:
                await page.set_viewport_size({'width': size, 'height': size})
                await page.set_content('<style>html,body{margin:0;background:transparent}svg{display:block;width:100vw;height:100vh}</style>' + source)
                frames[size] = await page.screenshot(omit_background=True)
        finally:
            await browser.close()
    (folder / 'shinyharbor.png').write_bytes(frames[256])
    sizes = [16, 24, 32, 48, 64, 128, 256]
    offset = 6 + 16 * len(sizes)
    header = struct.pack('<HHH', 0, 1, len(sizes)); body = b''
    for size in sizes:
        data = frames[size]
        header += struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        body += data; offset += len(data)
    (folder / 'shinyharbor.ico').write_bytes(header + body)
    chunks = b''
    for kind, size in [(b'ic07',128),(b'ic08',256),(b'ic09',512),(b'ic10',1024)]:
        data = frames[size]; chunks += kind + struct.pack('>I', len(data)+8) + data
    (folder / 'shinyharbor.icns').write_bytes(b'icns'+struct.pack('>I',len(chunks)+8)+chunks)
    print('Created SVG-derived PNG, multi-resolution ICO and ICNS icons.')

if __name__ == '__main__':
    asyncio.run(main())
