"""Inspect real DEB/RPM/Arch packages and execute each extracted binary."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def output(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True)


def main(directory):
    checks = []
    packages = [*directory.glob('*.deb'), *directory.glob('*.rpm'), *directory.glob('*.pkg.tar.zst')]
    assert len(packages) == 3, packages
    with tempfile.TemporaryDirectory(prefix='harbor-packages-') as temporary:
        root = Path(temporary)
        database = root / 'rpm-database'
        database.mkdir()
        subprocess.run(['rpm', '--dbpath', str(database), '--initdb'], check=True)

        def rpm(*args):
            return output('rpm', '--dbpath', database, *args)

        for number, package in enumerate(packages):
            digest = hashlib.sha256(package.read_bytes()).hexdigest()
            assert package.with_name(package.name + '.sha256').read_text().split()[0] == digest
            destination = root / str(number)
            destination.mkdir()
            if package.suffix == '.deb':
                assert output('dpkg-deb', '-f', package, 'Package').strip() == 'shinyharbor'
                assert 'r-cran-shiny' in output('dpkg-deb', '-f', package, 'Depends')
                subprocess.run(['dpkg-deb', '-x', str(package), str(destination)], check=True)
                controls = root / 'control'
                subprocess.run(['dpkg-deb', '-e', str(package), str(controls)], check=True)
                assert '/etc/shinyharbor/harbor.toml' in (controls / 'conffiles').read_text()
                for name in ['postinst', 'prerm', 'postrm']:
                    subprocess.run(['sh', '-n', str(controls / name)], check=True)
            elif package.suffix == '.rpm':
                assert rpm('-qp', '--qf', '%{NAME}', package).strip() == 'shinyharbor'
                assert 'R-shiny' in rpm('-qp', '--requires', package)
                names = rpm('-qpl', package).splitlines()
                assert '/etc/shinyharbor/harbor.toml' in names
                for tag in ['POSTIN', 'PREUN', 'POSTUN']:
                    script = root / (tag + '.sh')
                    script.write_text(rpm('-qp', '--qf', '%{' + tag + '}', package))
                    subprocess.run(['sh', '-n', str(script)], check=True)
                producer = subprocess.Popen(['rpm2cpio', str(package.resolve())], stdout=subprocess.PIPE)
                try:
                    subprocess.run(['cpio', '-id', '--quiet', '--no-absolute-filenames'], stdin=producer.stdout, cwd=destination, check=True)
                finally:
                    producer.stdout.close()
                assert producer.wait() == 0
            else:
                subprocess.run(['tar', '--zstd', '-xf', str(package), '-C', str(destination)], check=True)
                metadata = (destination / '.PKGINFO').read_text()
                assert 'pkgname = shinyharbor' in metadata and 'backup = etc/shinyharbor/harbor.toml' in metadata
                subprocess.run(['sh', '-n', str(destination / '.INSTALL')], check=True)
            executable = destination / 'usr/bin/shinyharbor'
            assert output(executable, '--version').startswith('shinyharbor ')
            assert (destination / 'usr/share/doc/shinyharbor/licenses').is_dir()
            config = (destination / 'etc/shinyharbor/harbor.toml').read_text()
            assert '/var/lib/shinyharbor' in config and '/srv/shinyharbor/apps' in config
            service = (destination / 'usr/lib/systemd/system/shinyharbor.service').read_text()
            assert 'User=shinyharbor' in service and 'ExecStart=/usr/bin/shinyharbor ' in service
            assert (destination / 'usr/share/shinyharbor/hello/app.R').is_file()
            checks.append(package.name + ': metadata, hooks, configuration, service, licenses and executable')
    report = {'suite': 'linux-packages', 'passed': True, 'checks': checks,
              'scope': 'Extracted package inspection/execution; does not claim distro installation lifecycle validation.'}
    Path('test-results').mkdir(exist_ok=True)
    Path('test-results/linux-packages.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--directory', type=Path, required=True)
    main(parser.parse_args().directory)
