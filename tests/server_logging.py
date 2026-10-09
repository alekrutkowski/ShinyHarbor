"""Run the gateway under original Shiny Server and Rust logging settings."""
import argparse
import asyncio
import json
import os
import sys
from pathlib import Path

from aiohttp import ClientSession
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory


async def exercise(binary):
    checks = []
    with TemporaryDirectory(prefix='harbor-server-logging-') as temp:
        root = Path(temp)
        public, admin = free_port(), free_port()
        while public == admin:
            admin = free_port()
        config = root / 'harbor.toml'
        settings = f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
        original = settings + "[[sites]]\nname='static'\nmount='/'\npath='.'\n"
        environment = {k: v for k, v in os.environ.items() if k not in ('RUST_LOG', 'SHINY_LOG_LEVEL')}
        for name, variables, info, warning in [
            ('default-INFO', {}, True, True),
            ('TRACE', {'SHINY_LOG_LEVEL': 'TRACE'}, True, True),
            ('DEBUG', {'SHINY_LOG_LEVEL': 'DEBUG'}, True, True),
            ('INFO', {'SHINY_LOG_LEVEL': 'INFO'}, True, True),
            ('WARN', {'SHINY_LOG_LEVEL': 'WARN'}, False, True),
            ('ERROR', {'SHINY_LOG_LEVEL': 'ERROR'}, False, False),
            ('RUST_LOG-suppresses-INFO', {'RUST_LOG': 'shinyharbor=error', 'SHINY_LOG_LEVEL': 'INFO'}, False, False),
            ('RUST_LOG-enables-INFO', {'RUST_LOG': 'shinyharbor=info', 'SHINY_LOG_LEVEL': 'ERROR'}, True, True),
        ]:
            config.write_text(original, encoding='utf-8')
            path = root / 'server.log'
            with path.open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config),
                    env=dict(environment, **variables), stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    token = (root / '.harbor/admin.token').read_text(encoding='utf-8').strip()
                    config.write_text(settings + 'unknown_setting=true\n', encoding='utf-8')
                    async with ClientSession() as client:
                        async with client.post(f'http://127.0.0.1:{admin}/api/reload',
                                               headers={'Authorization': 'Bearer ' + token}) as response:
                            assert response.status == 400
                except BaseException:
                    log.flush()
                    print(name, path.read_text(encoding='utf-8', errors='replace'))
                    raise
                finally:
                    if process.returncode is None:
                        process.terminate()
                    await asyncio.wait_for(process.wait(), 10)
            output = path.read_text(encoding='utf-8')
            assert ('ShinyHarbor started' in output) == info, (name, output)
            assert ('Configuration reload rejected' in output) == warning, (name, output)
            checks.append(name)
        command = await asyncio.create_subprocess_exec(str(binary.resolve()), 'check', '--config', str(config),
            env=dict(environment, SHINY_LOG_LEVEL='unknown'),
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        out, err = await asyncio.wait_for(command.communicate(), 10)
        assert command.returncode != 0 and b'SHINY_LOG_LEVEL must be' in out + err, (out, err)
        checks.append('invalid-level-has-actionable-diagnostic')
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    report = {'suite': 'server-logging', 'platform': sys.platform, 'passed': True, 'checks': checks}
    (results / f'server-logging-{sys.platform}.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(exercise(parser.parse_args().binary))
