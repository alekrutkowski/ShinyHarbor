#!/usr/bin/env python3
"""Build native installers from a checksum-verified portable release archive."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def write(path, content, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding='utf-8', newline='\n')
    path.chmod(mode)


def checksum(path):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    path.with_name(path.name + '.sha256').write_text(f'{digest}  {path.name}\n', encoding='ascii')


def payload(archive, destination):
    expected = archive.with_name(archive.name + '.sha256').read_text().split()[0]
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise ValueError('Portable archive checksum mismatch')
    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive) as source:
            for name in source.namelist():
                if Path(name).is_absolute() or '..' in Path(name).parts or '\\' in name:
                    raise ValueError('Unsafe archive member')
            source.extractall(destination)
    else:
        with tarfile.open(archive) as source:
            source.extractall(destination, filter='data')
    children = list(destination.iterdir())
    if len(children) != 1 or not children[0].is_dir():
        raise ValueError('Expected one portable release directory')
    return children[0]


def documentation(source, target):
    target.mkdir(parents=True, exist_ok=True)
    for name in ['LICENSE', 'THIRD_PARTY.md', 'sbom.cdx.json', 'README.md',
                 'VALIDATION.md', 'SECURITY.md', 'ATTRIBUTIONS.md']:
        shutil.copy2(source / name, target / name)
    for name in ['licenses', 'docs', 'packaging', 'validation', 'conformance', 'examples']:
        shutil.copytree(source / name, target / name)


PREPARE = '''for path in /etc/shinyharbor /etc/shinyharbor/harbor.toml /var/lib/shinyharbor /var/lib/shinyharbor/R /var/lib/shinyharbor/R/library /srv/shinyharbor /srv/shinyharbor/apps /srv/shinyharbor/apps/hello; do
  if [ -L "$path" ]; then
    echo "Refusing to change permissions through a symbolic link: $path" >&2
    exit 1
  fi
done
getent group shinyharbor >/dev/null || groupadd --system shinyharbor
id -u shinyharbor >/dev/null 2>&1 || useradd --system --gid shinyharbor --home-dir /var/lib/shinyharbor --shell /usr/sbin/nologin shinyharbor
install -d -o root -g shinyharbor -m 0770 /etc/shinyharbor
chown root:shinyharbor /etc/shinyharbor/harbor.toml
chmod 0660 /etc/shinyharbor/harbor.toml
install -d -o shinyharbor -g shinyharbor -m 0700 /var/lib/shinyharbor
install -d -o shinyharbor -g shinyharbor -m 0750 /var/lib/shinyharbor/R /var/lib/shinyharbor/R/library
install -d -m 0755 /srv/shinyharbor/apps/hello
/usr/bin/shinyharbor example --directory /srv/shinyharbor/apps/hello
'''
STOP = '''if [ -d /run/systemd/system ]; then
  systemctl stop shinyharbor.service || true
  systemctl disable shinyharbor.service || true
fi
'''


def linux(source, target, version, output, temporary):
    if not target.endswith('linux-musl'):
        raise ValueError('Linux native packages require a static musl release')
    machine = 'aarch64' if target.startswith('aarch64') else 'x86_64'
    deb_arch = 'arm64' if machine == 'aarch64' else 'amd64'
    native_version = version.replace('-', '~', 1)
    root = temporary / 'linux-root'
    (root / 'usr/bin').mkdir(parents=True)
    shutil.copy2(source / 'shinyharbor', root / 'usr/bin/shinyharbor')
    (root / 'usr/bin/shinyharbor').chmod(0o755)
    documentation(source, root / 'usr/share/doc/shinyharbor')
    write(root / 'etc/shinyharbor/harbor.toml', (ROOT / 'packaging/linux.toml').read_text())
    write(root / 'usr/lib/systemd/system/shinyharbor.service', (ROOT / 'packaging/shinyharbor.service').read_text())
    write(root / 'usr/share/shinyharbor/hello/app.R', (ROOT / 'examples/hello/app.R').read_text())
    write(root / 'usr/share/applications/shinyharbor.desktop', '[Desktop Entry]\nType=Application\nName=ShinyHarbor Applications\nExec=xdg-open http://127.0.0.1:3838/\nTerminal=false\nCategories=Development;\n')
    # Configuration is a conffile. App data/state is never deleted by hooks.
    deb = temporary / 'deb'
    shutil.copytree(root, deb)
    write(deb / 'DEBIAN/control', f'''Package: shinyharbor
Version: {native_version}-1
Architecture: {deb_arch}
Maintainer: Alek Rutkowski <alek.rutkowski@gmail.com>
Section: web
Priority: optional
Depends: r-base-core, r-cran-shiny, passwd, init-system-helpers
Homepage: https://github.com/alekrutkowski/ShinyHarbor
Description: Portable R Shiny application server
 HTTPS, R worker pools and a private usage dashboard.
''')
    write(deb / 'DEBIAN/conffiles', '/etc/shinyharbor/harbor.toml\n')
    write(deb / 'DEBIAN/postinst', '''#!/bin/sh
set -eu
[ "$1" = configure ] || exit 0
''' + PREPARE + '''if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  if [ -z "${2:-}" ]; then
    deb-systemd-helper enable shinyharbor.service >/dev/null
    deb-systemd-invoke start shinyharbor.service || true
  else
    deb-systemd-invoke try-restart shinyharbor.service || true
  fi
fi
echo 'ShinyHarbor: http://127.0.0.1:3838/ | config: /etc/shinyharbor/harbor.toml'
''', 0o755)
    write(deb / 'DEBIAN/prerm', '#!/bin/sh\nset -eu\nif [ "$1" = remove ]; then\n' + STOP + 'fi\n', 0o755)
    write(deb / 'DEBIAN/postrm', '''#!/bin/sh
set -eu
if [ "$1" = purge ]; then deb-systemd-helper purge shinyharbor.service >/dev/null || true; fi
if [ -d /run/systemd/system ]; then systemctl daemon-reload; fi
''', 0o755)
    # GitHub rewrites '~' in asset names. Keep native ordering in metadata,
    # but use the Cargo version (safe characters) in downloadable filenames.
    deb_file = output / f'shinyharbor_{version}-1_{deb_arch}.deb'
    run('dpkg-deb', '--root-owner-group', '--build', deb, deb_file)
    checksum(deb_file)

    rpm_top = temporary / 'rpmbuild'
    for folder in ['BUILD', 'BUILDROOT', 'RPMS', 'SOURCES', 'SPECS', 'SRPMS']:
        (rpm_top / folder).mkdir(parents=True)
    (rpm_top / 'database').mkdir()
    run('rpm', '--dbpath', rpm_top / 'database', '--initdb')
    # Native tools own RPM metadata, dependency declarations and scriptlets.
    spec = rpm_top / 'SPECS/shinyharbor.spec'
    write(spec, f'''%global debug_package %{{nil}}
%global __os_install_post %{{nil}}
Name: shinyharbor
Version: {native_version}
Release: 1
Summary: Portable R Shiny application server
License: MIT
URL: https://github.com/alekrutkowski/ShinyHarbor
BuildArch: {machine}
AutoReqProv: no
Requires: R-core, R-shiny, shadow-utils, systemd
%description
HTTPS, R worker pools and a private usage dashboard.
%install
mkdir -p "%{{buildroot}}"
cp -a "{root}/." "%{{buildroot}}/"
%post
{PREPARE}
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  if [ "$1" = 1 ]; then systemctl enable --now shinyharbor.service || true
  else systemctl try-restart shinyharbor.service || true; fi
fi
%preun
if [ "$1" = 0 ]; then
{STOP}fi
%postun
if [ -d /run/systemd/system ]; then systemctl daemon-reload; fi
%files
%config(noreplace) /etc/shinyharbor/harbor.toml
/usr/bin/shinyharbor
/usr/lib/systemd/system/shinyharbor.service
/usr/share/doc/shinyharbor
/usr/share/applications/shinyharbor.desktop
/usr/share/shinyharbor
''')
    run('rpmbuild', '--nodeps', '--dbpath', rpm_top / 'database', '--define', f'_dbpath {rpm_top / "database"}', '--define', f'_topdir {rpm_top}', '--define', '_rpmformat 4', '--target', machine, '-bb', spec)
    rpms = list((rpm_top / 'RPMS').rglob('*.rpm'))
    if len(rpms) != 1:
        raise RuntimeError('Expected one RPM')
    rpm_file = output / f'shinyharbor-{version}-1.{machine}.rpm'
    shutil.copy2(rpms[0], rpm_file)
    checksum(rpm_file)

    arch = temporary / 'arch'
    shutil.copytree(root, arch)
    arch_version = version.replace('-', '_')
    size = sum(path.stat().st_size for path in arch.rglob('*') if path.is_file())
    write(arch / '.PKGINFO', f'''pkgname = shinyharbor
pkgbase = shinyharbor
pkgver = {arch_version}-1
pkgdesc = Portable R Shiny application server
url = https://github.com/alekrutkowski/ShinyHarbor
builddate = {int(os.environ.get('SOURCE_DATE_EPOCH', time.time()))}
packager = Alek Rutkowski <alek.rutkowski@gmail.com>
size = {size}
arch = {machine}
license = MIT
depend = r
depend = systemd
depend = shadow
optdepend = gcc: compiling CRAN R packages
optdepend = make: compiling CRAN R packages
backup = etc/shinyharbor/harbor.toml
''')
    write(arch / '.INSTALL', 'post_install() {\n' + PREPARE + '''if [ -d /run/systemd/system ]; then systemctl daemon-reload; systemctl enable --now shinyharbor.service; fi
cat <<'HARBOR_SHINY_HELP'
Install Shiny once:
sudo -u shinyharbor Rscript -e 'install.packages("shiny", lib="/var/lib/shinyharbor/R/library", repos="https://cloud.r-project.org")'
HARBOR_SHINY_HELP
}
post_upgrade() {
if [ -d /run/systemd/system ]; then systemctl daemon-reload; systemctl try-restart shinyharbor.service; fi
}
pre_remove() {
''' + STOP + '}\n')
    arch_file = output / f'shinyharbor-{arch_version}-1-{machine}.pkg.tar.zst'
    # Explicit root ownership also makes rootless builds install correctly.
    run('tar', '--zstd', '--owner=0', '--group=0', '-cf', arch_file, '-C', arch, '.')
    checksum(arch_file)


def windows(source, target, version, output, compiler):
    architecture = 'arm64' if target.startswith('aarch64') else 'x64compatible'
    run(compiler, f'/DVersion={version}', f'/DArchitecture={architecture}', f'/DTarget={target}',
        f'/DPayload={source}', f'/DOutput={output}', ROOT / 'packaging/windows.iss')
    checksum(output / f'shinyharbor-{version}-{target}-setup.exe')


def macos(source, target, version, output, temporary):
    root = temporary / 'mac-root'
    (root / 'usr/local/bin').mkdir(parents=True)
    shutil.copy2(source / 'shinyharbor', root / 'usr/local/bin/shinyharbor')
    (root / 'usr/local/bin/shinyharbor').chmod(0o755)
    documentation(source, root / 'usr/local/share/doc/shinyharbor')
    for script in ['service.sh', 'uninstall.sh']:
        write(root / 'usr/local/libexec/shinyharbor' / script, (ROOT / 'packaging/macos' / script).read_text(), 0o755)
    shortcuts = {
        'Open applications.command': '#!/bin/sh\nexec /usr/local/bin/shinyharbor open apps\n',
        'Usage dashboard.command': '#!/bin/sh\nexec /usr/local/bin/shinyharbor open dashboard\n',
        'Server administration.command': '#!/bin/sh\nexec /usr/local/bin/shinyharbor open admin\n',
        'Terminal monitor.command': '#!/bin/sh\nexec /usr/local/bin/shinyharbor tui\n',
        'Start ShinyHarbor.command': '#!/bin/sh\nexec /usr/local/bin/shinyharbor\n',
        'Run at login.command': '#!/bin/sh\n/usr/local/libexec/shinyharbor/service.sh start\n',
        'Stop background server.command': '#!/bin/sh\n/usr/local/libexec/shinyharbor/service.sh stop\n',
        'Install Shiny R package.command': '#!/bin/sh\n/usr/local/bin/shinyharbor setup --install-shiny\nprintf "Press Enter to close."\nread answer\n',
        'Uninstall ShinyHarbor.command': '#!/bin/sh\n/usr/local/libexec/shinyharbor/service.sh stop\nsudo /usr/local/libexec/shinyharbor/uninstall.sh\n',
    }
    for name, content in shortcuts.items():
        write(root / 'Applications/ShinyHarbor' / name, content, 0o755)
    resources = temporary / 'mac-resources'
    write(resources / 'welcome.html', '<html><body><h1>Welcome to ShinyHarbor</h1><p>Installs the server and shortcuts in Applications/ShinyHarbor. Your apps and settings stay in your home folder and are preserved during upgrades and removal.</p><p>Install R from cran.r-project.org, then use Install Shiny R package and Start ShinyHarbor. Use Run at login for a background service.</p></body></html>')
    shutil.copy2(source / 'LICENSE', resources / 'license.txt')
    component = temporary / 'component.pkg'
    # Apple package versions are numeric; the public filename retains prerelease identity.
    numeric_version = version.split('-')[0]
    run('pkgbuild', '--root', root, '--identifier', 'org.shinyharbor.server', '--version', numeric_version,
        '--install-location', '/', '--ownership', 'recommended', component)
    architecture = 'arm64' if target.startswith('aarch64') else 'x86_64'
    distribution = temporary / 'Distribution.xml'
    write(distribution, f'''<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="1">
<title>ShinyHarbor {version}</title><welcome file="welcome.html"/><license file="license.txt"/>
<options customize="never" require-scripts="false" hostArchitectures="{architecture}"/>
<domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
<choices-outline><line choice="server"/></choices-outline>
<choice id="server" visible="false"><pkg-ref id="org.shinyharbor.server"/></choice>
<pkg-ref id="org.shinyharbor.server" version="{numeric_version}">component.pkg</pkg-ref>
</installer-gui-script>
''')
    package = output / f'shinyharbor-{version}-{target}.pkg'
    run('productbuild', '--distribution', distribution, '--resources', resources, '--package-path', temporary, package)
    checksum(package)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--iscc', default=str(ROOT / '.build/inno/ISCC.exe'))
    args = parser.parse_args()
    targets = json.loads((ROOT / 'tools/targets.json').read_text())['include']
    if args.target not in {item['target'] for item in targets}:
        parser.error('Unsupported target')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    extension = 'zip' if 'windows' in args.target else 'tar.gz'
    archive = args.archive or ROOT / 'dist' / f'shinyharbor-{version}-{args.target}.{extension}'
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-installer-') as directory:
        temporary = Path(directory)
        source = payload(archive.resolve(), temporary / 'payload')
        if 'windows' in args.target:
            windows(source, args.target, version, args.output.resolve(), args.iscc)
        elif 'apple' in args.target:
            macos(source, args.target, version, args.output.resolve(), temporary)
        else:
            linux(source, args.target, version, args.output.resolve(), temporary)


if __name__ == '__main__':
    main()
