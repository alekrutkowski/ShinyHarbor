"""Host-independent recipe checks; these do not claim native macOS execution."""
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('installers',ROOT/'tools/installers.py')
installers=importlib.util.module_from_spec(spec); spec.loader.exec_module(installers)

class InstallerRecipes(unittest.TestCase):
    def test_macos_system_service_and_launcher_use_the_system_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); source=root/'source'; source.mkdir()
            (source/'shinyharbor').write_bytes(b'test fixture, not an executable')
            (source/'LICENSE').write_text('test fixture')
            (source/'brand').mkdir(); (source/'brand/shinyharbor.icns').write_bytes(b'fixture')
            out=root/'out'; out.mkdir()
            calls=[]
            def run(*args):
                calls.append([str(a) for a in args])
                Path(args[-1]).write_bytes(b'recipe test fixture, not an installable package')
            with patch.object(installers,'run',run),patch.object(installers,'documentation'):
                installers.macos(source,'aarch64-apple-darwin','0.1.0-alpha.3',out,root)
            app=root/'mac-root/Applications/ShinyHarbor/ShinyHarbor.app/Contents'
            plist=plistlib.loads((app/'Info.plist').read_bytes())
            self.assertEqual(plist['CFBundleExecutable'],'ShinyHarbor')
            launcher=(app/'MacOS/ShinyHarbor').read_text()
            self.assertIn('with administrator privileges',launcher)
            self.assertIn('open apps --print --config /etc/shinyharbor/harbor.toml',launcher)
            self.assertIn('exec /usr/bin/open "$url"',launcher)
            self.assertIn('daemon.sh install',(root/'mac-scripts/postinstall').read_text())
            self.assertTrue(any('--scripts' in call for call in calls))
            service=plistlib.loads((root/'mac-root/usr/local/share/shinyharbor/org.shinyharbor.server.plist').read_bytes())
            self.assertEqual(service['UserName'],'_shinyharbor')
            self.assertTrue(service['RunAtLoad'])
            self.assertTrue(service['KeepAlive'])

if __name__=='__main__': unittest.main()
