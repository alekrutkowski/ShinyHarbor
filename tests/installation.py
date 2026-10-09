"""First-run setup, portable launch from another directory and upgrade preservation."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import sys

from aiohttp import ClientSession
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory


async def exercise(binary, check_r):
    binary = binary.resolve()
    checks = []
    with TemporaryDirectory(prefix='harbor-install-') as temp:
        root = Path(temp)
        program = root / 'portable program'
        program.mkdir()
        installed = program / binary.name
        shutil.copy2(binary, installed)
        elsewhere = root / 'unrelated working directory'
        elsewhere.mkdir()
        data = root / 'data 日本語 with spaces'
        environment = dict(os.environ)
        environment.pop('SHINYHARBOR_CONFIG', None)
        if not check_r:
            environment['LOCALAPPDATA'] = str(root / 'user')
        environment['PROGRAMDATA'] = str(root / 'shared')
        environment['XDG_DATA_HOME'] = str(root / 'user')

        async def command(*arguments, env=environment, expected=0):
            process = await asyncio.create_subprocess_exec(str(installed), *arguments, cwd=elsewhere, env=env,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
            out, err = await asyncio.wait_for(process.communicate(), 45)
            assert process.returncode == expected, (arguments, out, err)
            return out.decode('utf-8'), err.decode('utf-8')

        await command('setup', '--directory', str(data))
        config = data / 'harbor.toml'
        assert config.is_file() and (data / 'apps/hello/app.R').is_file()
        assert (data / 'R/library').is_dir()
        checks.append('setup-creates-config-example-and-R-library-in-selected-directory')
        original = config.read_text(encoding='utf-8') + '\n# retained user setting\n'
        config.write_text(original, encoding='utf-8')
        marker = data / 'apps/hello/my-data.txt'
        marker.write_text('keep this')
        await command('setup', '--directory', str(data))
        assert config.read_text(encoding='utf-8') == original and marker.read_text() == 'keep this'
        checks.append('repeat-setup-preserves-user-config-and-app-data')
        missing = dict(environment, SHINYHARBOR_CONFIG=str(root / 'missing.toml'))
        _, error = await command(env=missing, expected=1)
        assert 'Cannot find configuration' in error and 'setup' in error
        checks.append('missing-explicit-config-reports-path-and-remedy')

        ports = set()
        while len(ports) < 2:
            ports.add(free_port())
        public, admin = sorted(ports)
        source = original.replace('127.0.0.1:3838', f'127.0.0.1:{public}').replace('127.0.0.1:3839', f'127.0.0.1:{admin}')
        config.write_text(source, encoding='utf-8')
        (program / 'data-directory.txt').write_text(str(data), encoding='utf-8')

        async def serve():
            with (root / 'server.log').open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(installed), cwd=elsewhere, env=environment, stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    async with ClientSession() as client:
                        async with client.get(f'http://127.0.0.1:{public}/') as response:
                            assert response.status == 200 and 'hello' in await response.text()
                        if check_r:
                            async with client.get(f'http://127.0.0.1:{public}/hello/') as response:
                                assert response.status == 200, await response.text()
                except BaseException:
                    log.flush()
                    print((root / 'server.log').read_text(encoding='utf-8', errors='replace'))
                    for path in root.rglob('*-err.log'):
                        print(path.read_text(encoding='utf-8', errors='replace'))
                    raise
                finally:
                    if process.returncode is None:
                        process.terminate()
                    await asyncio.wait_for(process.wait(), 15)

        await serve()
        checks.append('bare-launch-follows-installer-data-pointer-with-Unicode-and-spaces')
        paths, _ = await command('paths')
        resolved = json.loads(paths)
        assert Path(resolved['config']).samefile(config)
        assert set(resolved) == {'config', 'state_dir', 'rscript', 'r_library'}
        checks.append('paths-command-resolves-installation-without-secrets')
        if check_r:
            no_path = dict(environment, PATH='')
            out, _ = await command('doctor', env=no_path)
            assert 'shiny ' in out
            checks.append('R-discovery-without-PATH-and-real-Shiny-startup')
        (program / 'data-directory.txt').unlink()
        shutil.copytree(data / 'apps', program / 'apps')
        (program / 'harbor.toml').write_text(source, encoding='utf-8')
        await serve()
        checks.append('bare-portable-executable-finds-adjacent-config-from-unrelated-directory')
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    report = {'suite': 'installation', 'platform': sys.platform, 'passed': True, 'checks': checks}
    (results / f'installation-{sys.platform}.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--r', action='store_true')
    args = parser.parse_args()
    asyncio.run(exercise(args.binary, args.r))
