#!/bin/sh
# Destructive package lifecycle test: disposable GitHub Linux runners only.
set -eu
[ "${GITHUB_ACTIONS:-}" = true ] || { echo 'Use a disposable GitHub runner.' >&2; exit 2; }
if dpkg-query -W shinyharbor >/dev/null 2>&1; then echo 'Existing installation; refusing test.' >&2; exit 2; fi
package=$(realpath "$1")
sudo apt-get install -y "$package"
sudo systemctl is-active --quiet shinyharbor
sudo systemctl is-enabled --quiet shinyharbor
sudo -u shinyharbor /usr/bin/shinyharbor doctor --config /etc/shinyharbor/harbor.toml
python3 -c 'import urllib.request; assert urllib.request.urlopen("http://127.0.0.1:3838/hello/", timeout=60).status == 200'
printf '\n# retained installer test setting\n' | sudo tee -a /etc/shinyharbor/harbor.toml >/dev/null
printf 'user app data\n' | sudo tee /srv/shinyharbor/apps/user-data.txt >/dev/null
sudo dpkg --force-confold -i "$package"
sudo systemctl is-active --quiet shinyharbor
grep -q 'retained installer test setting' /etc/shinyharbor/harbor.toml
sudo apt-get remove -y shinyharbor
test -f /etc/shinyharbor/harbor.toml
test -f /srv/shinyharbor/apps/user-data.txt
test ! -e /usr/bin/shinyharbor
mkdir -p test-results
printf '%s\n' '{"suite":"debian-installation-lifecycle","passed":true,"checks":["install-and-start","real-Shiny","upgrade-preserves-config","remove-preserves-data"]}' > test-results/debian-installation.json
