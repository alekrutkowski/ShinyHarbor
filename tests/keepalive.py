"""Socket inactivity, persistent requests and disabled timeouts on real listeners."""
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from gateway import free_port, wait_ready


async def request(reader, writer):
    writer.write(b'GET /file.txt HTTP/1.1\r\nHost: localhost\r\n\r\n')
    await writer.drain()
    head = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 2)
    assert head.startswith(b'HTTP/1.1 200'), head
    assert await reader.readexactly(5) == b'hello'


async def main(binary):
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-idle-') as temp:
        root = Path(temp)
        (root / 'site').mkdir()
        (root / 'site/file.txt').write_text('hello')
        public = free_port()
        config = root / 'server.conf'
        with (results / 'keepalive.log').open('wb') as log:
            for timeout in [.3, 0]:
                config.write_text(f'http_keepalive_timeout {timeout}; server {{ listen {public} 127.0.0.1; location / {{ site_dir site; }} }}')
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
                try:
                    await wait_ready(3839, process)
                    reader, writer = await asyncio.open_connection('127.0.0.1', public)
                    try:
                        for _ in range(8):
                            await request(reader, writer)
                            await asyncio.sleep(.09)
                        checks.append('reused-connection-survives-continued-traffic' if timeout else 'disabled-deadline-allows-persistent-requests')
                        if timeout:
                            assert await asyncio.wait_for(reader.read(1), 1.5) == b''
                            checks.append('idle-connection-closes-at-deadline')
                        else:
                            await asyncio.sleep(.6)
                            await request(reader, writer)
                            checks.append('zero-disables-idle-deadline')
                    finally:
                        writer.close()
                        await writer.wait_closed()
                    if timeout:
                        reader, writer = await asyncio.open_connection('127.0.0.1', public)
                        try:
                            writer.write(b'GET /file.txt HTTP/1.1\r\nHost:')
                            await writer.drain()
                            assert await asyncio.wait_for(reader.read(1), 1.5) == b''
                            checks.append('stalled-incomplete-request-closes')
                        finally:
                            writer.close()
                            await writer.wait_closed()
                finally:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'keepalive.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
