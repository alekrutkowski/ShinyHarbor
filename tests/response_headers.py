"""Public header policy, legacy migration and transactional live reload; no R required."""
import argparse
import asyncio
import json
import sys
from pathlib import Path

from aiohttp import ClientSession, ClientTimeout, web
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory


async def stop(process):
    if process and process.returncode is None:
        process.terminate()
        try:
            await asyncio.wait_for(process.wait(), 10)
        except asyncio.TimeoutError:
            process.kill()
            await process.wait()


async def exercise(binary):
    binary = str(binary.resolve())
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)

    async def backend(request):
        if request.path.endswith('/socket'):
            ws = web.WebSocketResponse()
            await ws.prepare(request)
            async for msg in ws:
                await ws.send_str(msg.data)
            return ws
        return web.Response(text='upstream response', headers={'X-Source': 'application'})

    app = web.Application()
    app.router.add_route('*', '/{tail:.*}', backend)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, '127.0.0.1', 0)
    await site.start()
    backend_port = site._server.sockets[0].getsockname()[1]
    try:
        with TemporaryDirectory(prefix='harbor-headers-') as temp:
            root = Path(temp)
            (root / 'index.html').write_text('static response', encoding='utf-8')
            ports = set()
            while len(ports) < 2:
                port = free_port()
                if port != 3839:
                    ports.add(port)
            public, admin = sorted(ports)
            base = f'http://127.0.0.1:{public}'
            config = root / 'harbor.toml'
            settings = (f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\n"
                        "state_dir='native-state'\nshutdown_grace_seconds=1\n")
            routes = (f"[[upstreams]]\nname='proxy'\nmount='/proxy'\nbackends=['http://127.0.0.1:{backend_port}']\n"
                      "[[sites]]\nname='static'\nmount='/site'\npath='.'\n"
                      "[[redirects]]\nname='old'\nmount='/old'\ntarget='/site/'\nstatus=302\nexact=true\n")

            def configured(headers):
                return settings + '[response_headers]\n' + ''.join(
                    json.dumps(k) + '=' + json.dumps(v) + '\n' for k, v in headers.items()) + routes

            original = configured({'X-Deployment': 'one', 'X-Removed': 'remove me',
                                   'X-Source': 'configuration', 'Content-Type': 'application/x-unwanted'})
            config.write_text(original, encoding='utf-8')
            with (results / f'response-headers-{sys.platform}.log').open('wb') as log:
                process = None
                try:
                    process = await asyncio.create_subprocess_exec(binary, 'serve', '--config', str(config), stdout=log, stderr=log)
                    await wait_ready(admin, process)
                    token = (root / 'native-state/admin.token').read_text(encoding='utf-8').strip()
                    async with ClientSession(timeout=ClientTimeout(total=10)) as client:
                        for path, status in [('/proxy/', 200), ('/site/', 200), ('/missing', 404), ('/old', 302), ('/proxy', 308)]:
                            async with client.get(base + path, allow_redirects=False) as response:
                                assert response.status == status, (path, response.status, await response.text())
                                assert response.headers['X-Deployment'] == 'one'
                                if status in (200, 404):
                                    assert response.headers['Content-Type'] != 'application/x-unwanted'
                                if path == '/proxy/':
                                    assert response.headers['X-Source'] == 'application'
                        checks += ['public-static-proxy-error-and-redirect-headers', 'application-and-server-headers-take-precedence']
                        for path in ['/', '/healthz', '/api/stats']:
                            async with client.get(f'http://127.0.0.1:{admin}' + path) as response:
                                assert 'X-Deployment' not in response.headers
                        checks.append('private-admin-policy-is-independent')

                        async def reload(status=200):
                            async with client.post(f'http://127.0.0.1:{admin}/api/reload',
                                                   headers={'Authorization': 'Bearer ' + token}) as response:
                                assert response.status == status, await response.text()

                        async with client.ws_connect(base + '/proxy/socket') as ws:
                            assert ws._response.headers['X-Deployment'] == 'one'
                            await ws.send_str('before reload')
                            assert (await ws.receive()).data == 'before reload'
                            checks.append('websocket-handshake-and-traffic')
                            config.write_text(configured({'X-Deployment': 'two'}), encoding='utf-8')
                            await reload()
                            async with client.get(base + '/proxy/') as response:
                                assert response.headers['X-Deployment'] == 'two'
                                assert 'X-Removed' not in response.headers
                            await ws.send_str('after reload')
                            assert (await ws.receive()).data == 'after reload'
                            checks.append('live-reload-updates-removes-headers-and-preserves-open-websocket')

                            invalid = [
                                {'Bad Name': 'invalid'}, {'X-Test': 'line\r\ninjection: value'},
                                {'Content-Length': '999'}, {'Connection': 'close'},
                                {'X-Test': 'one', 'x-test': 'two'},
                                {f'X-{n}': 'value' for n in range(65)}, {'X-Huge': 'x' * 16385},
                            ]
                            for headers in invalid:
                                config.write_text(configured(headers), encoding='utf-8')
                                await reload(400)
                                async with client.get(base + '/proxy/') as response:
                                    assert response.headers['X-Deployment'] == 'two'
                            await ws.send_str('after invalid reloads')
                            assert (await ws.receive()).data == 'after invalid reloads'
                            checks.append('invalid-name-value-framing-duplicates-and-budget-reloads-roll-back')
                finally:
                    await stop(process)

                legacy = root / 'shiny-server.conf'
                legacy_routes = f'server {{ listen {public} 127.0.0.1; location / {{ site_dir .; }} }}\n'
                legacy.write_text('set_header X-Legacy first;\nset_header x-legacy "last value";\n'
                                  'set_header Content-Type application/x-unwanted;\n' + legacy_routes, encoding='utf-8')
                process = None
                try:
                    process = await asyncio.create_subprocess_exec(binary, str(legacy), stdout=log, stderr=log)
                    await wait_ready(3839, process)
                    async with ClientSession(timeout=ClientTimeout(total=10)) as client:
                        async with client.get(base + '/') as response:
                            assert response.status == 200 and await response.text() == 'static response'
                            assert response.headers['X-Legacy'] == 'last value'
                            assert response.headers['Content-Type'].startswith('text/html')
                    checks.append('legacy-repeated-directives-case-insensitive-last-value-and-quoting')
                finally:
                    await stop(process)

                for invalid in ['set_header X-Invalid;\n' + legacy_routes,
                                legacy_routes.replace('location /', 'set_header X-Invalid yes; location /'),
                                legacy_routes.replace('site_dir .;', 'site_dir .; set_header X-Invalid yes;')]:
                    legacy.write_text(invalid, encoding='utf-8')
                    command = await asyncio.create_subprocess_exec(binary, 'check', '--config', str(legacy),
                        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                    out, err = await asyncio.wait_for(command.communicate(), 10)
                    assert command.returncode != 0 and b'set_header' in out + err, (out, err)
                checks.append('legacy-invalid-arity-and-nested-scopes-rejected')
    finally:
        await runner.cleanup()
    report = {'suite': 'response-headers', 'platform': sys.platform, 'passed': True, 'checks': checks}
    (results / f'response-headers-{sys.platform}.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    print(json.dumps(asyncio.run(exercise(parser.parse_args().binary)), indent=2))
