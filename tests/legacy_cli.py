"""Start the actual gateway through original and native CLI configuration forms."""
import argparse
import asyncio
import json
from pathlib import Path

from aiohttp import ClientSession
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory


async def main(binary):
    binary = binary.resolve()
    checks = []
    with TemporaryDirectory(prefix='harbor-legacy-cli-') as temporary:
        root = Path(temporary)
        site = root / 'site'
        site.mkdir()
        (site / 'index.html').write_text('original-config')
        public = free_port()
        config = root / 'server with spaces.conf'
        source = f'server {{ listen {public} 127.0.0.1; location / {{ site_dir "{site.as_posix()}"; }} }}'
        config.write_text(source, encoding='utf-8')

        async def serve(arguments, expected, admin=3839):
            with (root / 'server.log').open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(binary), *arguments,
                    cwd=root, stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    async with ClientSession() as client:
                        async with client.get(f'http://127.0.0.1:{public}/') as response:
                            assert response.status == 200, await response.text()
                            assert await response.text() == expected
                finally:
                    if process.returncode is None:
                        process.terminate()
                    await asyncio.wait_for(process.wait(), 15)

        await serve([str(config)], 'original-config')
        checks.append('original-positional-config-with-spaces')
        await serve(['--config', str(config)], 'original-config')
        checks.append('top-level-config-option')
        (root / 'shiny-server.conf').write_text(source, encoding='utf-8')
        await serve([], 'original-config')
        checks.append('local-legacy-default-config')
        command = await asyncio.create_subprocess_exec(str(binary), 'check', cwd=root,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        _, error = await command.communicate()
        assert command.returncode == 0, error.decode()
        checks.append('check-uses-same-default-search')
        native = root / 'native'
        native.mkdir()
        (native / 'index.html').write_text('native-config')
        admin = free_port()
        while admin == public:
            admin = free_port()
        (root / 'harbor.toml').write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\n"
            "[[sites]]\nname='native'\nmount='/'\npath='native'\n")
        await serve([], 'native-config', admin)
        checks.append('native-config-takes-precedence')
        for arguments in [[str(config), '--config', str(config)], ['--config', str(config), 'serve']]:
            command = await asyncio.create_subprocess_exec(str(binary), *arguments, cwd=root,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
            _, _ = await command.communicate()
            assert command.returncode == 2
        checks.append('ambiguous-config-arguments-rejected')
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    report = {'suite': 'legacy-cli', 'passed': True, 'checks': checks}
    (results / 'legacy-cli.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    asyncio.run(main(parser.parse_args().binary))
