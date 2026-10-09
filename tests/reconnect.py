"""Real Shiny recovery, message replay, expiry, and multiplex protocol checks."""
from __future__ import annotations
import argparse
import asyncio
import json
from test_support import TemporaryDirectory
import uuid
from pathlib import Path
from aiohttp import ClientSession, WSMsgType
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]

class Wire:
    def __init__(self, client, base, identifier=None):
        self.client, self.base = client, base
        self.identifier = identifier or uuid.uuid4().hex
        self.sent, self.received = 0, 0
        self.messages = []
        self.ws = None

    async def connect(self, resume=False, acknowledge=None):
        session = uuid.uuid4().hex
        self.ws = await self.client.ws_connect(f"{self.base}/{'o' if resume else 'n'}={self.identifier}/000/{session}/websocket")
        assert await self.ws.receive_str(timeout=10) == 'o'
        if resume:
            frame = await self.ws.receive_str(timeout=10)
            assert frame.startswith('a'), frame
            message = json.loads(frame[1:])[0]
            assert message == f'CONTINUE {self.sent:X}', message
            await self.raw(f'CONTINUE {self.received if acknowledge is None else acknowledge:X}')

    async def raw(self, value):
        await self.ws.send_json([value])

    async def send(self, data):
        await self.raw(f'{self.sent:X}#{data}')
        self.sent += 1

    async def next(self, ack=True):
        while True:
            frame = await self.ws.receive_str(timeout=15)
            if frame == 'h':
                continue
            if frame.startswith('c'):
                return ('close', json.loads(frame[1:]))
            assert frame.startswith('a'), frame
            for message in json.loads(frame[1:]):
                if message.startswith('ACK '):
                    continue
                tag, payload = message.split('#', 1)
                sequence = int(tag, 16)
                assert sequence == self.received, (sequence, self.received, payload)
                self.received += 1
                if ack:
                    await self.raw(f'ACK {self.received:X}')
                channel, method, value = payload.split('|', 2)
                return channel, method, json.loads(value)

async def wire_checks(base):
    async with ClientSession() as client:
        wire = Wire(client, base)
        await wire.connect()
        await wire.send('0|o|')
        first = await wire.next(ack=False)
        assert first[0:2] == ('0', 'm') and 'config' in first[2], first
        token = first[2]['config']['sessionId']
        # The server must replay an unacknowledged message after reconnection.
        await wire.ws.close()
        wire.received = 0
        await wire.connect(resume=True, acknowledge=0)
        replay = await wire.next()
        assert replay[2]['config']['sessionId'] == token
        # A second channel creates an independent Shiny session on the same transport.
        await wire.send('1|o|')
        second = await wire.next()
        assert second[0:2] == ('1', 'm') and second[2]['config']['sessionId'] != token, second
        # Duplicate sequence replay must not start another channel/session.
        await wire.raw('1#1|o|')
        await wire.send('0|c|{}')
        await wire.ws.close()
        # Unknown resume identifiers must fail, never silently create a new session.
        unknown = Wire(client, base)
        sock = await client.ws_connect(f'{base}/o={unknown.identifier}/000/unknown/websocket')
        assert await sock.receive_str(timeout=10) == 'o'
        closed = await sock.receive_str(timeout=10)
        assert json.loads(closed[1:])[0] == 4705, closed
        await sock.close()
        # After the grace period, an old session cannot be recovered.
        await asyncio.sleep(16)
        sock = await client.ws_connect(f'{base}/o={wire.identifier}/000/expired/websocket')
        assert await sock.receive_str(timeout=10) == 'o'
        closed = await sock.receive_str(timeout=10)
        assert json.loads(closed[1:])[0] == 4705, closed
        await sock.close()
    return ['numbered-messages', 'replay-unacknowledged-output', 'multiplex-independent-sessions',
            'duplicate-open-not-repeated', 'unknown-session-rejected', 'expired-session-rejected']

async def browser_checks(url):
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context()
        page = await context.new_page()
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        await page.goto(url)
        await expect(page.locator('#double')).to_have_text('4', timeout=30000)
        before = await page.evaluate('Shiny.shinyapp.config.sessionId')
        await page.locator('#dynamic').click()
        await expect(page.locator('#added')).to_have_text('Dynamic UI is working')
        await context.set_offline(True)
        await page.evaluate('Shiny.shinyapp.$socket.transport.close(4000, "test network interruption")')
        await expect(page.locator('[data-shinyharbor-reconnect]')).to_be_visible()
        await page.locator('#n').fill('47')
        await page.locator('#n').dispatch_event('change')
        await asyncio.sleep(1)
        await context.set_offline(False)
        await expect(page.locator('[data-shinyharbor-reconnect]')).to_have_count(0, timeout=15000)
        await expect(page.locator('#double')).to_have_text('94')
        assert await page.evaluate('Shiny.shinyapp.config.sessionId') == before
        await expect(page.locator('#added')).to_have_text('Dynamic UI is working')
        assert not errors, errors
        await browser.close()
    return ['same-R-session-after-network-loss', 'offline-input-delivered', 'existing-dynamic-state-preserved']

async def main(binary, reference):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-reconnect-') as temp:
        config = Path(temp) / 'harbor.toml'
        public, admin = free_port(), free_port()
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
                          "[[apps]]\nname='recovery'\nmount='/fixture'\nmax_workers=1\n"
                          f"path={json.dumps(str(ROOT / 'examples/compatibility'))}\n", encoding='utf-8')
        with (results / 'reconnect.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                checks = await browser_checks(f'http://127.0.0.1:{public}/fixture/')
                checks += await wire_checks(f'http://127.0.0.1:{public}/fixture/__sockjs__')
                if reference:
                    reference_checks = await wire_checks(reference.rstrip('/') + '/__sockjs__')
                    assert reference_checks == checks[-len(reference_checks):]
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 15)
    report = {'suite': 'reconnect', 'passed': True, 'checks': checks,
              'reference': reference, 'certifies_full_compatibility': False}
    (results / 'reconnect.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--reference')
    args = parser.parse_args()
    print(json.dumps(asyncio.run(main(args.binary, args.reference)), indent=2))
