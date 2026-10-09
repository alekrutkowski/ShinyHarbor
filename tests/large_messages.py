"""Large real Shiny inputs, outputs and replay, with enforced configured limits."""
import argparse
import asyncio
import json
from pathlib import Path

from aiohttp import ClientSession, ClientTimeout
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from reconnect import Wire
from test_support import TemporaryDirectory

SIZE = 3 * 1024 * 1024
APP = '''library(shiny)
ui <- fluidPage(
  tags$head(tags$script(HTML("$(document).on('shiny:connected',function(){Shiny.addCustomMessageHandler('large',function(x){document.getElementById('received').textContent=String(x.length);});});"))),
  tags$div(id='received', 'waiting'), textOutput('length'))
server <- function(input, output, session) {
  output$length <- renderText(nchar(if (is.null(input$payload)) '' else input$payload, type='bytes'))
  observeEvent(input$go, {
    session$sendCustomMessage('large', strrep('x', 3L * 1024L * 1024L))
  }, ignoreInit=TRUE)
}
shinyApp(ui, server)
'''


async def main(binary):
    binary = binary.resolve()
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-large-messages-') as temp:
        root = Path(temp)
        app = root / 'app'
        app.mkdir()
        (app / 'app.R').write_text(APP, encoding='utf-8')
        public, admin = free_port(), free_port()
        while public == admin:
            admin = free_port()
        config = root / 'harbor.toml'
        base = f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
        routes = ("[[apps]]\nname='large'\nmount='/app'\npath='app'\nmax_workers=1\n"
                  "[[apps]]\nname='polling'\nmount='/polling'\npath='app'\nmax_workers=1\n"
                  "disable_protocols=['websocket','streaming','jsonp-polling','iframe-xhr-polling']\n")
        config.write_text(base + routes, encoding='utf-8')
        with (results / 'large-messages.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async with async_playwright() as p:
                    browser = await p.chromium.launch(headless=True)
                    try:
                        for route in ['app', 'polling']:
                            page = await browser.new_page()
                            await page.goto(f'http://127.0.0.1:{public}/{route}/')
                            await expect(page.locator('#length')).to_have_text('0', timeout=60000)
                            await page.evaluate("n => { Shiny.setInputValue('payload', 'x'.repeat(n)); Shiny.setInputValue('go', 0); }", SIZE)
                            await expect(page.locator('#length')).to_have_text(str(SIZE), timeout=30000)
                            await page.evaluate("Shiny.setInputValue('go', 1)")
                            await expect(page.locator('#received')).to_have_text(str(SIZE), timeout=30000)
                            checks += [f'{route}-3MiB-input', f'{route}-3MiB-output']
                            await page.close()
                    finally:
                        await browser.close()
                async with ClientSession(timeout=ClientTimeout(total=30)) as client:
                    wire = Wire(client, f'http://127.0.0.1:{public}/app/__sockjs__')
                    await wire.connect()
                    await wire.send('0|o|')
                    assert 'config' in (await wire.next())[2]
                    await wire.send('0|m|' + json.dumps({'method': 'init', 'data': {'go': 0}}))
                    while 'values' not in (await wire.next())[2]:
                        pass
                    await wire.send('0|m|' + json.dumps({'method': 'update', 'data': {'go': 1}}))
                    while True:
                        _, _, message = await wire.next(ack=False)
                        if 'large' in message.get('custom', {}):
                            assert message['custom']['large'] == 'x' * SIZE
                            break
                        await wire.raw(f'ACK {wire.received:X}')
                    unacknowledged = wire.received - 1
                    await wire.ws.close()
                    wire.received = unacknowledged
                    await wire.connect(resume=True, acknowledge=unacknowledged)
                    replay = (await wire.next())[2]
                    assert replay['custom']['large'] == 'x' * SIZE
                    await wire.ws.close()
                    checks.append('3MiB-message-replayed-after-reconnect')
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)
            config.write_text(base + '[sockjs]\nmax_message_bytes=65536\n' + routes, encoding='utf-8')
            process = await asyncio.create_subprocess_exec(str(binary), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async with ClientSession(timeout=ClientTimeout(total=30)) as client:
                    url = f'http://127.0.0.1:{public}/app/__sockjs__/000/limit'
                    async with client.post(url + '/xhr') as response:
                        assert response.status == 200 and (await response.text()).startswith('o')
                    async with client.post(url + '/xhr_send', json=['x' * 65536]) as response:
                        assert response.status == 413
                    checks.append('configured-HTTP-message-limit-enforced')
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)
    report = {'suite': 'large-Shiny-messages', 'passed': True, 'checks': checks}
    (results / 'large-messages.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
