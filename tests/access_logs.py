"""Exercise real HTTP access logs, streamed responses and log rotation/reload."""
import argparse
import asyncio
import json
import re
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession, web
from gateway import free_port, wait_ready


async def lines(path, count):
    for _ in range(100):
        result = path.read_text().splitlines() if path.exists() else []
        if len(result) >= count:
            return result
        await asyncio.sleep(.03)
    raise AssertionError(f'Missing access logs in {path}: {result}')


async def main(binary):
    binary = binary.resolve()
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-access-') as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        (root / 'site').mkdir()
        (root / 'site/file.txt').write_text('hello')
        config = root / 'legacy.conf'
        config.write_text(f'access_log access.log; server {{ listen {public} 127.0.0.1; location / {{ site_dir site; }} }}')
        with (results / 'access-logs.log').open('wb') as output:
            process = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config), stdout=output, stderr=output)
            try:
                await wait_ready(3839, process)
                async with ClientSession() as client:
                    async with client.get(f'http://127.0.0.1:{public}/file.txt', headers={'X-Forwarded-For': '203.0.113.9', 'Authorization': 'Basic YWxpY2U6c2VjcmV0', 'User-Agent': 'quote"injected'}) as response:
                        assert await response.text() == 'hello'
                row = (await lines(root / 'access.log', 1))[0]
                assert re.match(r'127\.0\.0\.1 - alice \[.* GMT\] "GET /file.txt HTTP/1.1" 200 5 "-" "quote\\"injected"$', row), row
                assert 'secret' not in row and 'YWxpY2U' not in row and '203.0.113.9' not in row
                checks += ['legacy-default-format-and-append-file', 'peer-address-password-exclusion-and-escaping']
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)

            gate = asyncio.Event()
            async def streamed(request):
                response = web.StreamResponse(headers={'Content-Type': 'text/plain', 'X-Report': 'retained'})
                await response.prepare(request)
                await response.write(b'first')
                await gate.wait()
                await response.write(b'last')
                return response
            backend = web.Application()
            backend.router.add_get('/stream/', streamed)
            runner = web.AppRunner(backend)
            await runner.setup()
            site = web.TCPSite(runner, '127.0.0.1', 0)
            await site.start()
            port = site._server.sockets[0].getsockname()[1]
            config = root / 'harbor.toml'
            config.write_text(f'''listen = "127.0.0.1:{public}"
admin_listen = "127.0.0.1:{admin}"
shutdown_grace_seconds = 1
[access_log]
path = "custom.log"
format = ":method :url :status :req[x-marker] :res[x-report] :response-time[3] :total-time[3] :date[iso]"
[[upstreams]]
name = "stream"
mount = "/stream"
backends = ["http://127.0.0.1:{port}"]
''')
            process = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config), stdout=output, stderr=output)
            try:
                await wait_ready(admin, process)
                async with ClientSession(headers={'Accept-Encoding': 'identity'}) as client:
                    async with client.get(f'http://127.0.0.1:{public}/stream/', headers={'X-Marker': 'example'}) as response:
                        assert await response.content.readexactly(5) == b'first'
                        await asyncio.sleep(.3)
                        assert (root / 'custom.log').read_text() == ''
                        gate.set()
                        assert await response.read() == b'last'
                    row = (await lines(root / 'custom.log', 1))[0].split()
                    assert row[:5] == ['GET', '/stream/', '200', 'example', 'retained'], row
                    assert float(row[6]) > float(row[5]) + 200, row
                    assert re.match(r'\d{4}-\d\d-\d\dT.*Z$', row[7])
                    checks += ['custom-header-date-and-duration-tokens', 'streaming-not-buffered-and-logged-after-completion']
                    (root / 'custom.log').rename(root / 'rotated.log')
                    command = await asyncio.create_subprocess_exec(str(binary), 'reload', '--config', str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                    stdout, stderr = await command.communicate()
                    assert command.returncode == 0, (stdout, stderr)
                    async with client.get(f'http://127.0.0.1:{public}/missing') as response:
                        assert response.status == 404
                    assert 'GET /missing 404' in (await lines(root / 'custom.log', 1))[0]
                    assert len((root / 'rotated.log').read_text().splitlines()) == 1
                    checks.append('reload-reopens-rotated-log-and-errors-are-recorded')
                    token = (root / '.harbor/admin.token').read_text()
                    async with client.get(f'http://127.0.0.1:{admin}/metrics', headers={'Authorization': f'Bearer {token}'}) as response:
                        assert 'shinyharbor_dropped_access_logs 0' in await response.text()
                    checks.append('bounded-writer-loss-metric')
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)
                await runner.cleanup()

            for format in ['common', 'combined', 'short', 'tiny', 'dev', ':unknown']:
                config.write_text(f'listen = "127.0.0.1:{public}"\n[access_log]\npath = "named.log"\nformat = "{format}"\n[[sites]]\nname = "site"\nmount = "/"\npath = "site"\n')
                command = await asyncio.create_subprocess_exec(str(binary), 'check', '--config', str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                _, stderr = await command.communicate()
                assert (command.returncode == 0) == (format != ':unknown'), (format, stderr)
            checks.append('named-formats-and-invalid-token-validation')
    (results / 'access-logs.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
