#!/bin/sh
set -eu
[ "$(id -u)" = 0 ] || { echo 'Run with sudo.' >&2; exit 1; }
# Fixed package-owned paths only. Never remove anything in a user's home folder.
/usr/local/libexec/shinyharbor/daemon.sh stop
rm -f /Library/LaunchDaemons/org.shinyharbor.server.plist
rm -f /usr/local/bin/shinyharbor
rm -rf /Applications/ShinyHarbor /usr/local/share/doc/shinyharbor /usr/local/libexec/shinyharbor /usr/local/share/shinyharbor
pkgutil --forget org.shinyharbor.server >/dev/null
echo 'ShinyHarbor removed. User apps, settings and R libraries were preserved.'
