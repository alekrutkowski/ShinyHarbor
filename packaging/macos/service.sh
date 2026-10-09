#!/bin/sh
set -eu
label=org.shinyharbor.server
data="$HOME/Library/Application Support/ShinyHarbor"
plist="$HOME/Library/LaunchAgents/$label.plist"
domain="gui/$(id -u)"
case "${1:-}" in
  stop)
    launchctl bootout "$domain/$label" 2>/dev/null || true
    # Remove only our exact launch-agent file; application data is retained.
    rm -f "$plist"
    echo 'Background ShinyHarbor stopped. Your apps and settings are preserved.'
    ;;
  start)
    /usr/local/bin/shinyharbor setup --directory "$data"
    /usr/local/bin/shinyharbor doctor --config "$data/harbor.toml"
    mkdir -p "$HOME/Library/LaunchAgents" "$data/logs"
    saved_plist=$plist
    plist=$(mktemp "$HOME/Library/LaunchAgents/.$label.XXXXXX")
    trap 'rm -f "$plist"' EXIT HUP INT TERM
    printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict/></plist>' > "$plist"
    /usr/bin/plutil -insert Label -string "$label" "$plist"
    /usr/bin/plutil -insert ProgramArguments -array "$plist"
    /usr/bin/plutil -insert ProgramArguments.0 -string /usr/local/bin/shinyharbor "$plist"
    /usr/bin/plutil -insert ProgramArguments.1 -string serve "$plist"
    /usr/bin/plutil -insert ProgramArguments.2 -string --config "$plist"
    /usr/bin/plutil -insert ProgramArguments.3 -string "$data/harbor.toml" "$plist"
    /usr/bin/plutil -insert RunAtLoad -bool YES "$plist"
    /usr/bin/plutil -insert KeepAlive -dict "$plist"
    /usr/bin/plutil -insert KeepAlive.SuccessfulExit -bool NO "$plist"
    /usr/bin/plutil -insert StandardOutPath -string "$data/logs/service.log" "$plist"
    /usr/bin/plutil -insert StandardErrorPath -string "$data/logs/service.log" "$plist"
    /usr/bin/plutil -lint "$plist"
    mv -f "$plist" "$saved_plist"
    trap - EXIT HUP INT TERM
    plist=$saved_plist
    launchctl bootout "$domain/$label" 2>/dev/null || true
    launchctl bootstrap "$domain" "$plist"
    echo 'Running now and on future logins. Application address:'
    /usr/local/bin/shinyharbor open apps --print --config "$data/harbor.toml"
    ;;
  *) echo 'Usage: service.sh start|stop' >&2; exit 2;;
esac
