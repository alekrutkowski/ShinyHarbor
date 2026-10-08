"""Unix SIGHUP reload and invalid-configuration rollback, without a browser."""
import argparse
import asyncio
import json
import signal
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready


async def main(binary):
    with tempfile.TemporaryDirectory(prefix='harbor-signal-') as temp:
        root = Path(temp)
        site = root / 'site'
        site.mkdir()
        (site / 'index.html').write_text('reload fixture')
        public, admin = free_port(), free_port()
        config = root / 'harbor.toml'
        original = f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n[[sites]]\nname='fixture'\nmount='/first'\npath='site'\n"
        config.write_text(original)
        with (root / 'server.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async with ClientSession() as client:
                    config.write_text(original.replace("mount='/first'", "mount='/second'"))
                    process.send_signal(signal.SIGHUP)
                    async with asyncio.timeout(10):
                        while True:
                            async with client.get(f'http://127.0.0.1:{public}/second/') as response:
                                if response.status == 200:
                                    assert await response.text() == 'reload fixture'
                                    break
                            await asyncio.sleep(.1)
                    config.write_text('invalid = [')
                    process.send_signal(signal.SIGHUP)
                    # Wait for rejection to be recorded rather than assuming the
                    # old route still works before the signal was handled.
                    async with asyncio.timeout(10):
                        while 'reload rejected' not in (root / 'server.log').read_text():
                            await asyncio.sleep(.1)
                    async with client.get(f'http://127.0.0.1:{public}/second/') as response:
                        assert response.status == 200
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 15)
    report = {'checks': ['SIGHUP-route-reload', 'invalid-SIGHUP-keeps-working-configuration']}
    Path('test-results').mkdir(exist_ok=True)
    Path('test-results/reload-signal.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
