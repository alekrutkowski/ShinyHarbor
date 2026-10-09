#!/bin/sh
set -eu
[ "$(id -u)" = 0 ] || { echo 'Run with sudo.' >&2; exit 1; }
# Fixed package-owned paths only. Never remove anything in a user's home folder.
rm -f /usr/local/bin/shinyharbor
rm -rf /Applications/ShinyHarbor /usr/local/share/doc/shinyharbor /usr/local/libexec/shinyharbor
pkgutil --forget org.shinyharbor.server >/dev/null
echo 'ShinyHarbor removed. User apps, settings and R libraries were preserved.'
