"""Isolated Unix fixture pair for the real upstream bookmark migration test."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import sys
import tempfile

def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]

async def main(binary, reference_binary, serve_only):
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    stop = results / 'bookmark-fixture.stop'
    stop.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-bookmark-migration-') as temporary:
        root = Path(temporary)
        app = root / 'session'
        shutil.copytree('examples/session', app)
        for name in ['bookmarks', 'sockets', 'logs']:
            (root / name).mkdir(mode=0o700)
        first, second, admin = port(), port(), port()
        assert len({first, second, admin}) == 3
        account = pwd.getpwuid(os.geteuid()).pw_name
        reference = root / 'reference.conf'
        reference.write_text(f'''run_as {account};
socket_dir "{root / 'sockets'}";
bookmark_state_dir "{root / 'bookmarks'}";
server {{ listen {first} 127.0.0.1; location /fixture {{ app_dir "{app}"; log_dir "{root / 'logs'}"; }} }}
''')
        candidate = root / 'harbor.toml'
        candidate.write_text(f'''listen='127.0.0.1:{second}'
admin_listen='127.0.0.1:{admin}'
shutdown_grace_seconds=1
[[apps]]
name='migration'
mount='/fixture'
path='{app}'
bookmark_state_dir='{root / 'bookmarks'}'
''')
        processes = []
        logs = []
        try:
            for name, arguments in [('reference', [str(reference_binary.resolve()), str(reference)]),
                                    ('candidate', [str(binary.resolve()), 'serve', '--config', str(candidate)])]:
                log = Path(f'test-results/bookmark-{name}.log').open('wb')
                logs.append(log)
                processes.append(await asyncio.create_subprocess_exec(*arguments, stdout=log, stderr=log))
            for number in [first, second]:
                for attempt in range(100):
                    assert all(process.returncode is None for process in processes), 'Server failed; see test-results/bookmark-*.log'
                    try:
                        _, writer = await asyncio.open_connection('127.0.0.1', number)
                        writer.close()
                        await writer.wait_closed()
                        break
                    except OSError:
                        await asyncio.sleep(0.1)
                else:
                    raise RuntimeError('Server did not become ready')
            ready = {'reference': f'http://127.0.0.1:{first}/fixture/', 'candidate': f'http://127.0.0.1:{second}/fixture/'}
            (results / 'bookmark-fixture.json').write_text(json.dumps(ready))
            print(json.dumps(ready), flush=True)
            if serve_only:
                # A second process (possibly a Windows browser using WSL) runs
                # the client test, then creates the stop file. Bound the lifetime
                # so an interrupted controller cannot leave fixtures indefinitely.
                for _ in range(3000):
                    if stop.exists():
                        break
                    await asyncio.sleep(0.1)
            else:
                client = await asyncio.create_subprocess_exec(sys.executable,
                    'tests/bookmark_migration.py', '--reference', ready['reference'],
                    '--candidate', ready['candidate'])
                assert await asyncio.wait_for(client.wait(), 180) == 0, 'Bookmark migration failed'
        finally:
            for process in processes:
                if process.returncode is None:
                    process.terminate()
            for process in processes:
                try:
                    await asyncio.wait_for(process.wait(), 15)
                except asyncio.TimeoutError:
                    process.kill()
                    await process.wait()
            for log in logs:
                log.close()

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--reference-binary', type=Path, required=True)
    parser.add_argument('--serve-only', action='store_true')
    args = parser.parse_args()
    asyncio.run(main(args.binary, args.reference_binary, args.serve_only))
