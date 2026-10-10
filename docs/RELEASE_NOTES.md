# ShinyHarbor 0.1.0-alpha.3 — Reactive apps, new start page and Alpine

Sampling Lab now responds immediately to every input. The former **Run experiment**
button is gone; **Draw a new experiment** only changes the random seed. A fixed
seed keeps comparisons reproducible. CSV/PDF downloads track the current results.

This release adds a lighthouse icon, branded application catalog, Windows executable
and installer icons, Linux desktop icon and a prepared macOS application launcher.
The catalog keeps encoded app links, static-file access, private-file filtering and
custom directory templates.

Windows setup defaults to an automatic LocalService service, checks that the
installed executable supports `tui`, and grants access to configured leaf TLS files.
Per-user/portable foreground installations remain an explicit alternative. Linux
DEB/RPM/Arch use systemd. The new Alpine APK uses OpenRC; macOS packaging now prepares
a system LaunchDaemon running before login. Native macOS execution remains pending.

Compatibility additions: inherited `auth_ignore_case` user/group access matching
and `whitelist_headers` for additional Shiny session headers, including reconnect
channels. Credential usernames remain exact; reserved identity and transport headers
cannot be overridden. The README now explains every CLI command/option and includes
OS-specific service, HTTPS, upgrade and troubleshooting instructions.

## Installers in Assets and the local dist directory

| Platform | Installer |
| --- | --- |
| Windows x64 | `shinyharbor-0.1.0-alpha.3-x86_64-pc-windows-msvc-setup.exe` |
| Debian / Ubuntu x64 | `shinyharbor_0.1.0-alpha.3-1_amd64.deb` |
| Fedora x64 | `shinyharbor-0.1.0-alpha.3-1.x86_64.rpm` |
| Arch x64 | `shinyharbor-0.1.0_alpha.3-1-x86_64.pkg.tar.zst` |
| Alpine 3.23+ x64 (apk-tools 3) | `shinyharbor-0.1.0_alpha3-r0-x86_64.apk` |

Portable Windows ZIP and static Linux musl tar archives are also included. R and
app packages are separate dependencies. Alpine/Arch need the documented CRAN Shiny
installation step. Verify SHA-256 checksums; Windows and Alpine packages are unsigned.
No public package repository or automatic update channel is installed.

## Validation and remaining work

See [VALIDATION.md](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.3/VALIDATION.md)
for runtime checks and the release's `RELEASE_MANIFEST.json` for installer evidence
and exact hashes. Automatic startup configuration is checked without rebooting the
user's computer. Repository workflows include all eight OS/architecture targets
and Alpine service tests. The full current-source matrix has not passed; macOS and
ARM installers are not included in this manually published evaluation prerelease.

**Full Shiny Server and Professional drop-in compatibility is not achieved.**
Native PAM/LDAP/AD/OIDC/password-file login providers, complete legacy configuration
and live reload equivalence, and exhaustive protocol/platform conformance remain
open. No licensed Professional reference was available. Read the
[compatibility guide](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.3/docs/COMPATIBILITY.md)
before migration. HTTPS/ACME, statistics, administration and worker pools remain
available without paid feature limits; that is not a claim of full Professional parity.
