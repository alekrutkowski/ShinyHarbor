#!/bin/sh
# Only use in an isolated Alpine container/chroot, never on an existing install.
set -eu
[ "${SHINYHARBOR_ISOLATED_TEST:-}" = 1 ] || { echo 'Use an isolated Alpine test environment.' >&2; exit 2; }
if apk info -e shinyharbor >/dev/null 2>&1; then echo 'Existing installation; refusing test.' >&2; exit 2; fi
package=$(realpath "$1")
# This container shares its host's network; mark it available without running
# network configuration scripts against the host interfaces.
mkdir -p /run/openrc/started
ln -sf /etc/init.d/networking /run/openrc/started/networking
# The fixture may live on tmpfs. Its lifecycle ends with this test; it is not a
# diskless production installation that must restore packages after reboot.
apk add --root-tmpfs=no --allow-untrusted "$package"
test -L /etc/runlevels/default/shinyharbor
shinyharbor tui --help | grep -q -- --once
# Use free ports in shared-kernel chroots, where another host server may be running.
python3 - <<'PY'
import socket
from pathlib import Path
ports=[]
while len(set(ports)) < 2:
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); ports.append(s.getsockname()[1])
ports=list(dict.fromkeys(ports))
p=Path('/etc/shinyharbor/harbor.toml')
s=p.read_text().replace('127.0.0.1:3838',f'127.0.0.1:{ports[0]}').replace('127.0.0.1:3839',f'127.0.0.1:{ports[1]}')
p.write_text(s)
PY
shinyharbor check --config /etc/shinyharbor/harbor.toml
# A container has no system boot. Initialize only OpenRC's marker and exercise
# the actual service supervisor; --nodeps avoids manipulating host networking.
mkdir -p /run/openrc
echo default > /run/openrc/softlevel
rc-service --nodeps shinyharbor start
trap 'rc-service --nodeps shinyharbor stop || true' EXIT HUP INT TERM
rc-service shinyharbor status
python3 - <<'PY'
import pwd, time, urllib.request, tomllib
from pathlib import Path
address=tomllib.loads(Path('/etc/shinyharbor/harbor.toml').read_text())['listen']
for attempt in range(60):
    try:
        with urllib.request.urlopen('http://'+address+'/',timeout=2) as r:
            assert r.status == 200 and b'Your applications' in r.read()
        break
    except OSError:
        if attempt==59: raise
        time.sleep(.5)
uid=pwd.getpwnam('shinyharbor').pw_uid
assert uid != 0
workers=[]
for process in Path('/proc').glob('[0-9]*'):
    try:
        command=(process/'cmdline').read_bytes().split(b'\0')
        if command[:4] == [b'/usr/bin/shinyharbor',b'serve',b'--config',b'/etc/shinyharbor/harbor.toml']:
            workers.append((process/'status').read_text())
    except (OSError, PermissionError):
        continue
assert workers and all(f'Uid:\t{uid}\t{uid}\t{uid}\t{uid}' in status for status in workers)
assert Path('/etc/shinyharbor/harbor.toml').stat().st_mode & 0o007 == 0
PY
printf '\n# preserved Alpine test setting\n' >> /etc/shinyharbor/harbor.toml
printf 'preserved app data\n' > /srv/shinyharbor/apps/keep.txt
python3 - <<'PY'
import subprocess, sys, tempfile, tomllib
from pathlib import Path
sys.path.insert(0,str(Path('tools').resolve()))
import installers
machine=subprocess.check_output(['apk','--print-arch'],text=True).strip()
target=f'{machine}-unknown-linux-musl'
version=tomllib.loads(Path('Cargo.toml').read_text())['package']['version']
with tempfile.TemporaryDirectory(prefix='harbor-upgrade-test-') as directory:
    temporary=Path(directory)
    source=installers.payload(Path(f'dist/shinyharbor-{version}-{target}.tar.gz'),temporary/'payload')
    # The same payload with a higher package revision exercises apk's real
    # post-upgrade hook. This fixture is temporary and is never released.
    installers.alpine(source,target,version,temporary,temporary,revision=1)
    package=next(temporary.glob('*.apk'))
    Path('/usr/bin/shinyharbor').unlink()
    subprocess.run(['apk','add','--root-tmpfs=no','--allow-untrusted','--upgrade',str(package)],check=True)
PY
test -x /usr/bin/shinyharbor
test "$(stat -c '%U:%G:%a' /usr/bin/shinyharbor)" = 'root:root:755'
grep -q 'preserved Alpine test setting' /etc/shinyharbor/harbor.toml
test -L /etc/runlevels/default/shinyharbor
rc-service shinyharbor status
rc-service --nodeps shinyharbor stop
trap - EXIT HUP INT TERM
apk del --root-tmpfs=no shinyharbor
test ! -e /usr/bin/shinyharbor
test ! -e /etc/runlevels/default/shinyharbor
test -f /srv/shinyharbor/apps/keep.txt
test -d /var/lib/shinyharbor
mkdir -p test-results
printf '%s\n' '{"suite":"alpine-installation","passed":true,"checks":["native-APK-install","OpenRC-boot-registration","unprivileged-supervised-start","real-HTTP-catalog","CLI-TUI-help","real-upgrade-preserves-config","root-owned-executable","service-stop","remove-preserves-app-and-state"]}' > test-results/alpine-installation.json
