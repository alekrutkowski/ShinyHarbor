"""Repeatable gateway load probe; reports measurements, not a performance guarantee."""
import argparse
import asyncio
import ctypes
import json
import os
import platform
import statistics
import tempfile
import time
from pathlib import Path
from aiohttp import ClientSession, TCPConnector, web
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]

def memory(pid):
    if os.name == 'nt':
        class Counters(ctypes.Structure):
            _fields_ = [('size',ctypes.c_ulong), ('faults',ctypes.c_ulong)] + [(name,ctypes.c_size_t) for name in
                ('peak','resident','peak_paged','paged','peak_nonpaged','nonpaged','pagefile','peak_pagefile')]
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        psapi = ctypes.WinDLL('psapi', use_last_error=True)
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        psapi.GetProcessMemoryInfo.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ulong]
        handle = kernel.OpenProcess(0x410, False, pid)
        try:
            values = Counters()
            values.size = ctypes.sizeof(values)
            if not handle or not psapi.GetProcessMemoryInfo(handle, ctypes.byref(values), values.size):
                raise ctypes.WinError(ctypes.get_last_error())
            return values.resident
        finally:
            if handle:
                kernel.CloseHandle(handle)
    if platform.system() == 'Linux':
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith('VmRSS:'):
                return int(line.split()[1]) * 1024
    return None

async def main(binary, requests, concurrency):
    assert requests > 0 and 0 < concurrency <= requests
    payload = b'x' * 1024
    async def handler(_):
        return web.Response(body=payload, content_type='text/plain')
    app = web.Application()
    app.router.add_get('/{path:.*}', handler)
    runner = web.AppRunner(app)
    await runner.setup()
    backend = free_port()
    await web.TCPSite(runner, '127.0.0.1', backend).start()
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    samples = []
    latencies = []
    with tempfile.TemporaryDirectory(prefix='harbor-load-') as temp:
        config = Path(temp) / 'harbor.toml'
        public, admin = free_port(), free_port()
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
            f"[[upstreams]]\nname='probe'\nmount='/probe'\nbackends=['http://127.0.0.1:{backend}']\n")
        with (results / 'load.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, process)
                async def sample():
                    while True:
                        samples.append(memory(process.pid))
                        await asyncio.sleep(0.1)
                sampler = asyncio.create_task(sample())
                async with ClientSession(connector=TCPConnector(limit=concurrency)) as client:
                    async def worker(number):
                        for _ in range(number, requests, concurrency):
                            started = time.perf_counter()
                            async with client.get(f'http://127.0.0.1:{public}/probe/') as response:
                                assert response.status == 200 and await response.read() == payload
                            latencies.append((time.perf_counter() - started) * 1000)
                    started = time.perf_counter()
                    await asyncio.gather(*(worker(i) for i in range(concurrency)))
                    elapsed = time.perf_counter() - started
                sampler.cancel()
                try:
                    await sampler
                except asyncio.CancelledError:
                    pass
            finally:
                if process.returncode is None:
                    process.terminate()
                    await asyncio.wait_for(process.wait(), 15)
                await runner.cleanup()
    latencies.sort()
    report = {'suite':'gateway-load-probe', 'passed':True, 'platform':platform.platform(),
        'binary':str(binary), 'requests':requests, 'concurrency':concurrency, 'body_bytes':len(payload),
        'analytics_enabled':True, 'errors':0, 'elapsed_seconds':round(elapsed,3),
        'requests_per_second':round(requests/elapsed,1), 'p50_ms':round(statistics.median(latencies),3),
        'p95_ms':round(latencies[int(0.95*(len(latencies)-1))],3),
        'peak_observed_RSS_MiB':round(max(s for s in samples if s is not None)/1024**2,2) if any(s is not None for s in samples) else None,
        'scope':'Loopback 1 KiB responses via Python aiohttp backend/client; does not measure R application capacity or isolated gateway throughput.'}
    (results / 'load.json').write_text(json.dumps(report, indent=2))
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--requests', default=10000, type=int)
    parser.add_argument('--concurrency', default=32, type=int)
    args = parser.parse_args()
    print(json.dumps(asyncio.run(main(args.binary, args.requests, args.concurrency)), indent=2))
