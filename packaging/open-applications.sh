#!/bin/sh
# Keep the browser in the desktop account; only reading configuration needs sudo.
set -eu
url=$(sudo /usr/bin/shinyharbor open apps --print --config /etc/shinyharbor/harbor.toml)
exec xdg-open "$url"
