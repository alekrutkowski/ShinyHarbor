"""Real R exit-code retention, preserved logs, permissions and stderr relay."""
import argparse
import asyncio
import json
import os
import tempfile
from pathlib import Path
from aiohttp import ClientSession, ClientError
from gateway import free_port, wait_ready


async def main(binary, account=None):
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    checks = []
    with tempfile.TemporaryDirectory(prefix='harbor-logs-') as temp:
        root = Path(temp)
        app = root / 'app'
        app.mkdir()
        user = None
        if account:
            import pwd
            assert os.geteuid() == 0
            user = pwd.getpwnam(account)
            assert user.pw_uid != 0
            root.chmod(0o755)
            owned = root / 'owned'
            owned.mkdir()
            os.chown(owned, user.pw_uid, user.pw_gid)
            owned.chmod(0o700)
        (app / 'app.R').write_text('''library(shiny)
cat("worker stdout marker\\n")
message("worker stderr marker")
shinyApp(function(request) {
  query <- request$QUERY_STRING
  if (is.null(query)) query <- ""
  if (startsWith(query, "?")) query <- substring(query, 2L)
  if (identical(query, "bulk")) cat(strrep("x", 11L * 1024L * 1024L))
  if (!is.null(query) && grepl("^quit=[0-9]+$", query)) quit(save="no", status=as.integer(sub("quit=", "", query)), runLast=FALSE)
  fluidPage("ready")
}, function(input, output, session) {})
''')
        for number, (preserve, status) in enumerate([(False, 0), (True, 0), (False, 3)]):
            public, admin = free_port(), free_port()
            logdir = root / f'logs-{number}'
            if account:
                logdir = root / 'owned' / f'logs-{number}'
            config = root / 'harbor.toml'
            config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n[[apps]]\nname='fixture'\nmount='/app'\npath='app'\nlog_dir='{logdir.as_posix()}'\nlog_file_mode='0620'\npreserve_logs={str(preserve).lower()}\n" + (f'log_as_user=true\nrun_as=[{json.dumps(account)}]\n' if account else ''))
            output = results / f'worker-logs-{number}.log'
            with output.open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), env={**os.environ, 'SHINY_LOG_STDERR': '1'}, stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    async with ClientSession() as client:
                        async with client.get(f'http://127.0.0.1:{public}/app/') as response:
                            assert response.status == 200
                        async with asyncio.timeout(10):
                            while len(list(logdir.glob('*.log'))) != 2:
                                await asyncio.sleep(.1)
                        if os.name != 'nt':
                            assert all(p.stat().st_mode & 0o777 == 0o620 for p in logdir.glob('*.log'))
                        if user:
                            assert all(p.stat().st_uid == user.pw_uid and p.stat().st_gid == user.pw_gid for p in logdir.glob('*.log'))
                            assert logdir.stat().st_uid == user.pw_uid
                        assert 'worker stdout marker' in output.read_text()
                        assert 'worker stderr marker' in output.read_text()
                        if user and status == 3:
                            async with client.get(f'http://127.0.0.1:{public}/app/?bulk') as response:
                                assert response.status == 200
                            async with asyncio.timeout(10):
                                while not list(logdir.glob('*.log.1')):
                                    await asyncio.sleep(.1)
                            assert all(p.stat().st_uid == user.pw_uid and p.stat().st_size <= 10 * 1024 * 1024 for p in logdir.glob('*.log*'))
                            checks.append('worker-account-rotates-bounded-log-files')
                        try:
                            async with client.get(f'http://127.0.0.1:{public}/app/?quit={status}') as response:
                                assert response.status >= 500, (response.status, await response.text())
                        except ClientError:
                            pass
                        token = (root / '.harbor/admin.token').read_text()
                        async with asyncio.timeout(15):
                            while True:
                                async with client.get(f'http://127.0.0.1:{admin}/api/status', headers={'Authorization': 'Bearer ' + token}) as response:
                                    if (await response.json())['pools'][0]['workers'] == 0:
                                        break
                                await asyncio.sleep(.1)
                        await asyncio.sleep(.2)
                        retained = list(logdir.glob('*.log'))
                        assert bool(retained) == (preserve or status != 0), retained
                        assert len(list(logdir.glob('*.keep'))) == (2 if preserve else 0)
                        checks.append(f'preserve={preserve}:exit={status}:retention-and-relay')
                finally:
                    if process.returncode is None:
                        process.terminate()
                        await asyncio.wait_for(process.wait(), 20)
    if os.name != 'nt':
        checks.append('Unix-log-mode-applied-regardless-of-umask')
    if account:
        checks.append('worker-account-creates-owns-and-deletes-logs-and-directory')
    (results / ('user-logs.json' if account else 'worker-logs.json')).write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--account')
    args = parser.parse_args()
    asyncio.run(main(args.binary, args.account))
