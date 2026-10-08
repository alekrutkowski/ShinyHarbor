#!/bin/sh
# Install an already downloaded release archive after an explicit SHA-256 check.
set -eu
if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "Usage: sh install.sh ARCHIVE.tar.gz EXPECTED_SHA256 [DESTINATION]" >&2
  exit 2
fi
archive=$1
expected=$2
destination=${3:-"$HOME/.local/bin"}
case "$expected" in *[!a-fA-F0-9]*|'') echo 'Invalid SHA-256 digest' >&2; exit 2;; esac
[ "${#expected}" -eq 64 ] || { echo 'SHA-256 must have 64 hex digits' >&2; exit 2; }
if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$archive" | cut -d ' ' -f 1)
else actual=$(shasum -a 256 "$archive" | cut -d ' ' -f 1); fi
[ "$actual" = "$(printf '%s' "$expected" | tr A-F a-f)" ] || { echo 'Checksum mismatch; nothing installed' >&2; exit 1; }
# Never extract arbitrary entries as root. Read only the expected executable member.
member=$(tar -tzf "$archive" | grep -E '^shinyharbor-[^/]+/shinyharbor$')
[ "$(printf '%s\n' "$member" | wc -l | tr -d ' ')" -eq 1 ] || { echo 'Unexpected archive layout' >&2; exit 1; }
mkdir -p "$destination"
temp=$(mktemp "$destination/.shinyharbor.XXXXXX")
trap 'rm -f "$temp"' EXIT HUP INT TERM
tar -xzOf "$archive" "$member" > "$temp"
chmod 755 "$temp"
mv -f "$temp" "$destination/shinyharbor"
echo "Installed $destination/shinyharbor. Keep the release's LICENSE, THIRD_PARTY.md and licenses/ when redistributing."
