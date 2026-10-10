#!/bin/sh
# System LaunchDaemon: runs after reboot without a user login.
set -eu
[ "$(id -u)" = 0 ] || { echo 'Use sudo for system service management.' >&2; exit 1; }
label=org.shinyharbor.server
plist=/Library/LaunchDaemons/org.shinyharbor.server.plist
data='/Library/Application Support/ShinyHarbor'
config=/etc/shinyharbor/harbor.toml
case "${1:-}" in
  stop)
    launchctl bootout "system/$label" 2>/dev/null || true
    launchctl disable "system/$label"
    echo 'Service stopped and disabled. Applications and settings are preserved.'
    exit 0;;
  status) exec launchctl print "system/$label";;
  restart)
    launchctl enable "system/$label"
    exec launchctl kickstart -k "system/$label";;
  install|start) ;;
  *) echo 'Usage: sudo daemon.sh install|start|restart|stop|status' >&2; exit 2;;
esac
for path in /etc/shinyharbor "$config" "$data" "$data/apps" "$data/apps/hello" "$data/state" "$data/R" "$data/R/library" "$plist"; do
  [ ! -L "$path" ] || { echo "Refusing symbolic link: $path" >&2; exit 1; }
done
if ! id _shinyharbor >/dev/null 2>&1; then
  # Pick an unused system UID/GID; never reuse another account's identity.
  account_id=300
  while dscl . -list /Users UniqueID | awk '{print $2}' | grep -qx "$account_id" || dscl . -list /Groups PrimaryGroupID | awk '{print $2}' | grep -qx "$account_id"; do
    account_id=$((account_id + 1))
    [ "$account_id" -lt 500 ] || { echo 'No free system account ID below 500.' >&2; exit 1; }
  done
  if dscl . -read /Groups/_shinyharbor >/dev/null 2>&1; then
    echo 'An existing _shinyharbor group requires administrator review.' >&2; exit 1
  fi
  dscl . -create /Groups/_shinyharbor
  dscl . -create /Groups/_shinyharbor PrimaryGroupID "$account_id"
  dscl . -create /Users/_shinyharbor
  dscl . -create /Users/_shinyharbor UniqueID "$account_id"
  dscl . -create /Users/_shinyharbor PrimaryGroupID "$account_id"
  dscl . -create /Users/_shinyharbor UserShell /usr/bin/false
  dscl . -create /Users/_shinyharbor NFSHomeDirectory "$data"
  dscl . -create /Users/_shinyharbor RealName 'ShinyHarbor service'
  dscl . -create /Users/_shinyharbor IsHidden 1
else
  [ "$(dscl . -read /Users/_shinyharbor NFSHomeDirectory | cut -d ' ' -f 2-)" = "$data" ] || { echo 'Existing service account has an unexpected home.' >&2; exit 1; }
  account_id=$(id -u _shinyharbor)
  [ "$account_id" -ge 300 ] && [ "$account_id" -lt 500 ] || { echo 'Unexpected service UID; refusing to run as this account.' >&2; exit 1; }
  [ "$(dscl . -read /Users/_shinyharbor UserShell | awk '{print $2}')" = /usr/bin/false ] || { echo 'Unexpected service account shell.' >&2; exit 1; }
fi
install -d -o root -g _shinyharbor -m 0770 /etc/shinyharbor
install -d -o root -g _shinyharbor -m 0755 "$data" "$data/apps" "$data/apps/hello"
install -d -o _shinyharbor -g _shinyharbor -m 0700 "$data/state"
install -d -o _shinyharbor -g _shinyharbor -m 0750 "$data/R" "$data/R/library"
if [ ! -f "$config" ]; then
  install -o root -g _shinyharbor -m 0660 /usr/local/share/shinyharbor/macos.toml "$config"
fi
chown root:_shinyharbor "$config"
chmod 0660 "$config"
/usr/local/bin/shinyharbor example --directory "$data/apps/hello"
/usr/local/bin/shinyharbor check --config "$config"
install -o root -g wheel -m 0644 /usr/local/share/shinyharbor/org.shinyharbor.server.plist "$plist"
launchctl bootout "system/$label" 2>/dev/null || true
launchctl enable "system/$label"
launchctl bootstrap system "$plist"
echo 'System service installed. It starts after reboot without a user login.'
/usr/local/bin/shinyharbor open apps --print --config "$config"
