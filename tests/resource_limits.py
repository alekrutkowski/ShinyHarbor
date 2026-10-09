"""Real R worker scaling, overload recovery and per-session idle limits."""
from __future__ import annotations
import argparse
import asyncio
import json
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready
from reconnect import Wire

ROOT = Path(__file__).resolve().parents[1]


async def value(wire, expected):
    for _ in range(20):
        message = await asyncio.wait_for(wire.next(), 5)
        assert message[:2] == ('0', 'm'), message
        if message[:2] == ('0', 'm') and message[2].get('values', {}).get('double') == str(expected):
            return
    raise AssertionError('Expected reactive output was not received')


async def exercise(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-limits-') as temp:
        root = Path(temp)
        public = free_port()
        config = root / 'server.conf'
        config.write_text(f'''run_as :PROCESS_USER:;
sockjs_heartbeat_delay 1;
server {{ listen {public} 127.0.0.1;
  location /pool {{ app_dir "{(ROOT / 'examples/compatibility').as_posix()}"; utilization_scheduler 4 .5 2; }}
  location /idle {{ app_dir "{(ROOT / 'examples/compatibility').as_posix()}"; app_session_timeout 2; }}
  location /plain {{ app_dir "{(ROOT / 'examples/compatibility').as_posix()}"; app_session_timeout 2; reconnect false; }}
}}
''', encoding='utf-8')
        with (results / 'resource-limits.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, process)
                token = (root / '.harbor/admin.token').read_text()
                base = f'http://127.0.0.1:{public}'
                async with ClientSession() as client:
                    async def workers():
                        async with client.get('http://127.0.0.1:3839/api/status', headers={'Authorization': 'Bearer ' + token}) as response:
                            data = await response.json()
                            return next(p['workers'] for p in data['pools'] if p['mount'] == '/pool')
                    sessions = []
                    for number in range(8):
                        wire = Wire(client, base + '/pool/__sockjs__')
                        await wire.connect()
                        await wire.send('0|o|')
                        assert 'config' in (await wire.next())[2]
                        sessions.append(wire)
                        assert await workers() == (1 if number < 2 else 2)
                    full = Wire(client, base + '/pool/__sockjs__')
                    await full.connect()
                    await full.send('0|o|')
                    assert (await full.next())[:2] == ('0', 'c')
                    await full.ws.close()
                    checks += ['load-factor-spawns-second-worker', 'maximum-worker-count', 'overload-rejected']
                    await sessions[0].send('0|c|{}')
                    await asyncio.sleep(.5)
                    replacement = Wire(client, base + '/pool/__sockjs__')
                    await replacement.connect()
                    await replacement.send('0|o|')
                    assert 'config' in (await replacement.next())[2]
                    checks.append('closed-session-restores-capacity')
                    for wire in [*sessions, replacement]:
                        await wire.ws.close()
                    idle = Wire(client, base + '/idle/__sockjs__')
                    await idle.connect()
                    await idle.send('0|o|')
                    assert 'config' in (await idle.next())[2]
                    assert (await asyncio.wait_for(idle.next(), 5))[:2] == ('0', 'c')
                    await idle.ws.close()
                    checks.append('robust-idle-session-closed-despite-heartbeats')
                    active = Wire(client, base + '/idle/__sockjs__')
                    await active.connect()
                    await active.send('0|o|')
                    await active.next()
                    await active.send('0|m|' + json.dumps({'method': 'init', 'data': {'n': 2, '.clientdata_output_double_hidden': False}}))
                    await value(active, 4)
                    for n in range(3, 8):
                        await asyncio.sleep(.7)
                        await active.send('0|m|' + json.dumps({'method': 'update', 'data': {'n': n}}))
                        await value(active, 2 * n)
                    for _ in range(20):
                        message = await asyncio.wait_for(active.next(), 5)
                        if message[:2] == ('0', 'c'):
                            break
                    else:
                        raise AssertionError('Active session did not expire after data stopped')
                    await active.ws.close()
                    checks.append('application-data-resets-session-idle-timer')
                    async with client.ws_connect(base + '/plain/__sockjs__/000/idle/websocket') as ws:
                        assert await ws.receive_str(timeout=5) == 'o'
                        while True:
                            frame = await ws.receive_str(timeout=5)
                            if frame.startswith('c'):
                                break
                    checks.append('plain-SockJS-idle-session-closed')
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'resource-limits.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(exercise(parser.parse_args().binary))
