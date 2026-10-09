"""Real R worker profiles, supervisor arguments and inherited configuration."""
import argparse
import asyncio
import html
import json
import os
from pathlib import Path
import re
import shlex
import sys

from aiohttp import ClientSession, ClientTimeout
from gateway import free_port, wait_ready
from test_support import TemporaryDirectory


async def main(binary):
    binary = binary.resolve()
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-worker-environment-') as temp:
        root = Path(temp)
        app = root / "app space ' & Unicode"
        app.mkdir()
        (app / 'app.R').write_text('''library(shiny)
shinyApp(fluidPage(
  tags$pre(id='profile', Sys.getenv('HARBOR_TEST_PROFILE', 'absent')),
  tags$pre(id='supervisor', Sys.getenv('HARBOR_TEST_SUPERVISOR', 'absent')),
  tags$pre(id='server', jsonlite::toJSON(shiny::serverInfo(), auto_unbox=TRUE)),
  tags$pre(id='port', Sys.getenv('SHINY_PORT')),
  tags$pre(id='working', normalizePath(getwd(), winslash='/'))),
  function(input, output, session) {})
''', encoding='utf-8')
        supervisor = root / 'supervisor with spaces.py'
        supervisor.write_text('''import os, sys, subprocess
os.environ['HARBOR_TEST_SUPERVISOR'] = sys.argv[1]
if os.name == 'nt':
    sys.exit(subprocess.run(sys.argv[2:]).returncode)
os.execvp(sys.argv[2], sys.argv[2:])
''', encoding='utf-8')
        # Metacharacters must remain literal arguments, including under Bash.
        argument = "literal ; $(echo unexpected) & 'quoted'"
        command = [Path(sys.executable).as_posix(), supervisor.as_posix(), argument]
        home = root / 'profile-home'
        home.mkdir()
        for name, value in [('.bash_profile', 'first'), ('.bash_login', 'second'), ('.profile', 'third')]:
            (home / name).write_text(f"export HARBOR_TEST_PROFILE={value}\ncd /\n")
        environment = os.environ.copy()
        environment['HOME'] = str(home)
        # Preserve the test account's R package search paths after changing HOME.
        if os.name != 'nt' and 'R_LIBS_USER' not in environment:
            query = await asyncio.create_subprocess_exec('Rscript', '-e',
                'cat(paste(.libPaths(), collapse=.Platform$path.sep))', stdout=asyncio.subprocess.PIPE)
            output, _ = await query.communicate()
            assert query.returncode == 0
            environment['R_LIBS_USER'] = output.decode().strip()
        public, admin = free_port(), free_port()
        while public == admin:
            admin = free_port()
        config = root / 'harbor.toml'
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
            "[[apps]]\nname='environment'\nmount='/supervised'\n"
            f"path={json.dumps(str(app))}\nsupervisor={json.dumps(command)}\n", encoding='utf-8')

        async def exercise(config, admin, expected, cleared=False):
            with (results / f'worker-environment-{sys.platform}.log').open('ab') as log:
                process = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config),
                    env=environment, stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    async with ClientSession(timeout=ClientTimeout(total=60)) as client:
                        for route in (['supervised', 'clear'] if cleared else ['supervised']):
                            async with client.get(f'http://127.0.0.1:{public}/{route}/') as response:
                                body = await response.text()
                                if response.status != 200:
                                    for path in (root / '.harbor/logs').glob('*.log'):
                                        print(path.name, path.read_text(errors='replace'))
                                assert response.status == 200, body
                                fields = {key: html.unescape(value) for key, value in re.findall(r'<pre id="([^"]+)">([^<]*)</pre>', body)}
                                assert fields['profile'] == expected, fields
                                assert fields['working'].casefold() == app.resolve().as_posix().casefold(), fields
                                assert fields['supervisor'] == (argument if route == 'supervised' else 'absent'), fields
                                info = json.loads(fields['server'])
                                assert info['shinyServer'] is True and info['edition'] == 'ShinyHarbor', info
                                assert info['version'] != 'development', info
                                assert 0 < int(fields['port']) < 65536, fields
                finally:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)

        await exercise(config, admin, 'absent')
        checks += ['portable-supervisor-argv', 'literal-metacharacters', 'TOML-direct-startup']
        checks.append('hosted-runtime-metadata-before-app-evaluation')
        config = root / 'shiny-server.conf'
        # Two distinct quoting layers: the command's argv and the legacy directive.
        prefix = shlex.join(command)
        quote = lambda value: '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'
        config.write_text(f'''exec_supervisor {quote(prefix)};
server {{ listen {public} 127.0.0.1;
location /supervised {{ app_dir {quote(app.as_posix())}; }}
location /clear {{ app_dir {quote(app.as_posix())}; exec_supervisor ""; }}
}}''', encoding='utf-8')
        await exercise(config, 3839, 'first' if os.name != 'nt' else 'absent', cleared=True)
        checks += ['legacy-supervisor-inheritance', 'empty-supervisor-clears-inheritance']
        if os.name != 'nt':
            checks += ['legacy-Bash-login-profile', 'working-directory-restored-after-profile']
            (home / '.bash_profile').unlink()
            await exercise(config, 3839, 'second')
            (home / '.bash_login').unlink()
            await exercise(config, 3839, 'third')
            checks.append('Bash-profile-precedence-and-fallback')
    report = {'suite': 'worker-environment', 'passed': True, 'platform': sys.platform, 'checks': checks}
    (results / f'worker-environment-{sys.platform}.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
