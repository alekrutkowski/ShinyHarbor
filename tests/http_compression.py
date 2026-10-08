"""HTTP encoding negotiation, with range and disabled-mode checks."""
import argparse
import asyncio
import gzip
import json
import tempfile
import zlib
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready


async def main(binary):
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-compression-') as temp:
        root = Path(temp)
        (root / 'site').mkdir()
        data = ('A representative static document.\n' * 2048).encode()
        (root / 'site/large.txt').write_bytes(data)
        (root / 'site/small.txt').write_text('small')
        for enabled in (True, False):
            public = free_port()
            config = root / 'server.conf'
            config.write_text(f"http_allow_compression {'on' if enabled else 'off'};\nserver {{ listen {public} 127.0.0.1; location / {{ site_dir site; }} }}\n")
            with (results / f'compression-{enabled}.log').open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
                try:
                    await wait_ready(3839, process)
                    async with ClientSession(auto_decompress=False) as client:
                        base = f'http://127.0.0.1:{public}'
                        for coding, decode in (('gzip', gzip.decompress), ('deflate', zlib.decompress)):
                            async with client.get(base + '/large.txt', headers={'Accept-Encoding': coding}) as response:
                                body = await response.read()
                                assert response.status == 200
                                if enabled:
                                    assert response.headers['Content-Encoding'] == coding
                                    assert len(body) < len(data) // 2
                                    assert decode(body) == data
                                    assert 'accept-encoding' in response.headers['Vary'].lower()
                                else:
                                    assert 'Content-Encoding' not in response.headers
                                    assert body == data
                            checks.append(f'{coding}-' + ('negotiation-and-content' if enabled else 'disabled'))
                        async with client.get(base + '/large.txt', headers={'Range': 'bytes=10-39', 'Accept-Encoding': 'gzip'}) as response:
                            assert response.status == 206 and 'Content-Encoding' not in response.headers
                            assert await response.read() == data[10:40]
                        async with client.get(base + '/small.txt', headers={'Accept-Encoding': 'gzip'}) as response:
                            assert 'Content-Encoding' not in response.headers and await response.text() == 'small'
                        checks.append(f'ranges-and-small-body-{enabled}')
                finally:
                    if process.returncode is None:
                        process.terminate()
                        await asyncio.wait_for(process.wait(), 20)
    (results / 'compression.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
