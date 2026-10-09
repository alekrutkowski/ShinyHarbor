"""Unix PTY: real raw-mode keyboard handling, terminal restoration, server survival."""
import argparse
import json
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import tempfile
import time

def run(binary):
    import fcntl
    import pty
    import termios
    with tempfile.TemporaryDirectory(prefix='harbor-tui-') as tmp:
        root=Path(tmp)
        def port():
            with socket.socket() as s: s.bind(('127.0.0.1',0)); return s.getsockname()[1]
        public,admin=port(),port()
        while public==admin: admin=port()
        config=root/'harbor.toml'
        config.write_text(f'listen="127.0.0.1:{public}"\nadmin_listen="127.0.0.1:{admin}"\nshutdown_grace_seconds=1\n[[sites]]\nname="site"\nmount="/"\npath="."\ndirectory_index=true\n')
        with (root/'server.log').open('wb') as log:
            server=subprocess.Popen([str(binary),'serve','--config',str(config)],stdout=log,stderr=log)
            try:
                for _ in range(100):
                    if (root/'.harbor/admin.token').exists(): break
                    assert server.poll() is None
                    time.sleep(.05)
                master,slave=pty.openpty()
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,110,0,0))
                before=termios.tcgetattr(slave)
                tui=subprocess.Popen([str(binary),'tui','--config',str(config)],stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color'})
                try:
                    def until(text):
                        data=b''; deadline=time.monotonic()+10
                        while text not in data and time.monotonic()<deadline:
                            if select.select([master],[],[],.2)[0]: data+=os.read(master,65536)
                        assert text in data, data[-2000:]
                    until(b'SHINYHARBOR')
                    os.write(master,b'2'); until(b'APPLICATIONS')
                    os.write(master,b'3'); until(b'KEYBOARD')
                    os.write(master,b'Q'); assert tui.wait(timeout=10)==0
                    assert termios.tcgetattr(slave)==before
                    assert server.poll() is None
                finally:
                    if tui.poll() is None: tui.kill(); tui.wait()
                    os.close(master); os.close(slave)
            finally:
                server.terminate(); server.wait(timeout=15)
    result={'suite':'terminal-pty','passed':True,'checks':['raw-keyboard-navigation','uppercase-Q-quits','terminal-settings-restored','server-remains-running']}
    Path('test-results').mkdir(exist_ok=True)
    Path('test-results/terminal-pty.json').write_text(json.dumps(result,indent=2))
    print(json.dumps(result,indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--binary',type=Path,required=True)
    run(parser.parse_args().binary.resolve())
