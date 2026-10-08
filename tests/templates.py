"""Original fixture templates exercising the upstream Handlebars data contract."""
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready


async def main(binary):
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    checks = []
    with tempfile.TemporaryDirectory(prefix='harbor-templates-') as temp:
        root = Path(temp)
        site = root / 'site'
        site.mkdir()
        (site / 'folder').mkdir()
        (site / 'app').mkdir()
        (site / 'app/app.R').write_text('stop("Must not start R merely to list this app")')
        (site / 'a & b.txt').write_text('public')
        (site / 'private.log').write_text('private')
        (site / 'PRIVATE.LOG').write_text('private')
        template = root / 'templates'
        template.mkdir()
        directory = template / 'directoryIndex.html'
        directory.write_text('<h1>{{title}}</h1>{{#each apps}}<a class="app" href="{{this.url}}">{{this.name}}</a>{{/each}}{{#each dirs}}<i>{{this.name}}</i>{{/each}}{{#each files}}<b>{{this.name}}</b>{{/each}}')
        (template / 'error.html').write_text('<h1>Custom {{title}}</h1>{{#if message}}<p>{{message}}</p>{{/if}}')
        public = free_port()
        config = site / 'server.conf'
        config.write_text(f'template_dir ../templates; access_log access.txt; server {{ listen {public} 127.0.0.1; location /site {{ site_dir .; directory_index on; }} }}')
        with (results / 'templates.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, process)
                async with ClientSession() as client:
                    base = f'http://127.0.0.1:{public}'
                    async with client.get(base + '/site/') as response:
                        body = await response.text()
                        assert response.status == 200
                        assert '<h1>Index of /site/</h1>' in body and '<i>app</i>' in body
                        assert '<i>folder</i>' in body and '<b>a &amp; b.txt</b>' in body
                        assert 'private.log' not in body and 'PRIVATE.LOG' not in body
                        assert 'access.txt' not in body and 'server.conf' not in body
                    checks += ['inherited-directory-template', 'app-directory-file-context', 'HTML-escaping-and-hidden-logs']
                    for private in ('access.txt', 'server.conf', '.harbor/admin.token'):
                        async with client.get(base + '/site/' + private) as response:
                            assert response.status == 404, private
                    checks.append('access-log-configuration-and-state-never-served-as-static-files')
                    async with client.get(base + '/missing') as response:
                        assert response.status == 404
                        assert '<h1>Custom Not Found</h1>' in await response.text()
                        assert response.headers['Content-Type'].startswith('text/html')
                    async with client.head(base + '/missing') as response:
                        assert response.status == 404 and await response.read() == b''
                    checks.append('error-template-preserves-status-and-HEAD')
                    (template / 'error-404.html').write_text('<h2>Specific missing page</h2>')
                    async with client.get(base + '/missing') as response:
                        assert '<h2>Specific missing page</h2>' in await response.text()
                    checks.append('specific-error-template-overrides-fallback')
                    directory.write_text('<h2>Updated template</h2>')
                    async with client.get(base + '/site/') as response:
                        assert '<h2>Updated template</h2>' in await response.text()
                    checks.append('template-file-reloads-without-service-restart')
                    directory.write_text('{{#each files}}missing end')
                    command = await asyncio.create_subprocess_exec(str(binary.resolve()), 'check', '--config', str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                    _, stderr = await command.communicate()
                    assert command.returncode != 0 and b'Invalid template' in stderr
                    checks.append('invalid-template-rejected-by-config-check')
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 20)
    (results / 'templates.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
