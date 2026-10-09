"""Real Shiny HTTP and both WebSocket bridges over Unix domain sockets."""
import argparse
import asyncio
import json
import os
import stat
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession, UnixConnector
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]


async def value(ws, framed=False):
    for _ in range(40):
        message = await asyncio.wait_for(ws.receive_str(), 5)
        values = json.loads(message[1:]) if framed and message.startswith('a') else [message]
        for value in values:
            if value in ('o', 'h'):
                continue
            data = json.loads(value)
            if data.get('values', {}).get('double') == '42':
                return
    raise AssertionError('Reactive output not received')


async def main(binary):
    assert os.name == 'posix', 'Unix socket test needs Linux or macOS'
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-uds-', dir='/tmp') as temp:
        root = Path(temp)
        public = free_port()
        config = root / 'server.conf'
        config.write_text(f'''socket_dir sockets;
reconnect false;
server {{ listen {public} 127.0.0.1;
location /fixture {{ app_dir "{ROOT / 'examples/compatibility'}"; }}
}}''')
        with (results / 'unix-socket.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, process)
                base = f'http://127.0.0.1:{public}/fixture'
                async with ClientSession() as client:
                    async with client.get(base + '/') as response:
                        body = await response.text()
                        assert response.status == 200 and 'ShinyHarbor browser fixture' in body, body
                    checks.append('legacy-socket-directory-real-R-HTTP')
                    sockets = list((root / 'sockets').glob('*/r.sock'))
                    assert len(sockets) == 1 and stat.S_ISSOCK(sockets[0].stat().st_mode)
                    assert sockets[0].stat().st_mode & 0o077 == 0
                    assert sockets[0].parent.stat().st_mode & 0o077 == 0
                    checks.append('private-domain-socket-and-worker-directory')
                    async with ClientSession(connector=UnixConnector(path=str(sockets[0]))) as direct:
                        async with direct.get('http://localhost/') as response:
                            assert response.status == 403
                    checks.append('direct-socket-request-requires-worker-secret')
                    async with client.ws_connect(base + '/websocket/') as ws:
                        assert 'config' in await ws.receive_json()
                        await ws.send_json({'method': 'init', 'data': {'n': 21, '.clientdata_output_double_hidden': False}})
                        await value(ws)
                    checks.append('native-upgraded-HTTP-reactivity-over-domain-socket')
                    async with client.ws_connect(base + '/__sockjs__/000/uds-test/websocket') as ws:
                        assert await ws.receive_str() == 'o'
                        await ws.send_json([json.dumps({'method': 'init', 'data': {'n': 21, '.clientdata_output_double_hidden': False}})])
                        await value(ws, framed=True)
                    checks.append('SockJS-reactivity-over-domain-socket')
            except Exception:
                for path in (root / '.harbor/logs').glob('*.log'):
                    print(path.name, path.read_text())
                raise
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)
            assert list((root / 'sockets').iterdir()) == []
            checks.append('worker-sockets-cleaned-on-shutdown')
    report = {'checks': checks}
    (results / 'unix-socket.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
