# Installing ShinyHarbor

Choose the installer matching your operating system and CPU. Local builds are in
`dist/`; the release workflow also produces these installers as GitHub artifacts.
Windows x64 and Linux x64 packages have been built locally. macOS and ARM
installer builds remain pending while GitHub hosted CI is blocked by account billing.
Rust, Cargo and a C compiler are not needed to install the server executable.
R and Shiny are separate runtime dependencies. The current installers are alpha
builds; see VALIDATION.md for platforms actually exercised.

| System | Installer | Installation |
| --- | --- | --- |
| Windows x64 / ARM64 | `*-setup.exe` | Double-click; select program and data folders |
| Debian / Ubuntu x64 / ARM64 | `*.deb` | `sudo apt install ./shinyharbor_*.deb` |
| Fedora x64 / ARM64 | `*.rpm` | `sudo dnf install ./shinyharbor-*.rpm` |
| Arch x64 / Arch Linux ARM | `*.pkg.tar.zst` | `sudo pacman -U ./shinyharbor-*.pkg.tar.zst` |
| macOS Intel / Apple Silicon | `*.pkg` | Double-click; open Applications → ShinyHarbor |

Use one architecture-specific file in each command. These are installable local
packages; the project does not yet operate an APT/DNF repository or publish an AUR
entry. Linux packages use the static musl build and require systemd. ARM packages
use aarch64/ARM64, not 32-bit ARM. Package manager dependencies use the distribution's
R/Shiny versions; CRAN upgrades are described below.

## Windows

Run the setup wizard. The default installation is for your account and needs no
administrator rights. You can select an all-users installation when a background
Windows service is wanted. Both the program directory and the data directory can
be changed, for example to `Z:\ShinyHarbor` and `Z:\ShinyHarborData`.

Install R from <https://cran.r-project.org/> first. ShinyHarbor discovers ordinary
R installations without requiring PATH edits. The optional Shiny checkbox downloads
the package and its dependencies into the chosen data folder. The background-service
option also installs this private R library so the service does not depend on your
personal R library.

The Start menu contains **Start ShinyHarbor**, **Open applications**, **Usage
dashboard**, **Edit configuration**, and **Application folders**. For a foreground
server, keep its console open. The service option uses LocalService, starts on boot,
and requires local disks: mapped drives such as a network-backed Z: belong to your
login session and are unsuitable for that service account. Use the foreground
shortcut for a mapped drive. The service requires `state_dir` to be a subfolder of
the selected data directory and does not accept junctions in that data path.

Upgrades preserve `harbor.toml`, applications and R packages. Apps & Features
uninstallation removes the program and its service, preserving your data directory.
An existing service belonging to another installation is not modified.

Unattended installation is available:

```powershell
.\shinyharbor-VERSION-x86_64-pc-windows-msvc-setup.exe /CURRENTUSER /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR="Z:\ShinyHarbor" /DATADIR="Z:\ShinyHarborData"
```

The Windows installer is currently unsigned; signing requires a publisher's
code-signing certificate. The release includes SHA-256 checksums.

## Linux

Install the matching local package with APT, DNF or pacman. Debian/Ubuntu and Fedora
also install their packaged R and Shiny dependencies. Arch installs R; install the
Shiny package once under the service account:

```sh
sudo -u shinyharbor Rscript -e 'install.packages("shiny", lib="/var/lib/shinyharbor/R/library", repos="https://cloud.r-project.org")'
```

Compiling CRAN packages on Arch needs the standard `base-devel` tools. To use
current CRAN Shiny on Debian/Ubuntu/Fedora, the same command installs it into the
private library, which takes precedence over the distribution's library. Native
build tools may be needed if binary R packages are unavailable.

The installer starts `shinyharbor.service` when systemd is running. It binds to
loopback initially. Configuration: `/etc/shinyharbor/harbor.toml`; apps:
`/srv/shinyharbor/apps`; private state and R library: `/var/lib/shinyharbor`.

```sh
systemctl status shinyharbor
sudo journalctl -u shinyharbor -f
sudo shinyharbor admin-token --config /etc/shinyharbor/harbor.toml
sudo systemctl restart shinyharbor
```

Package upgrades preserve edited configuration and app files. Removing the package
stops the service and retains app/state directories; package-manager purge may
remove configuration, but never recursively deletes your app data. The service
account is retained for file ownership. Configure native HTTPS or a reverse proxy
before exposing the listener to other machines.

## macOS

Install the Intel or Apple Silicon `.pkg`. Install R from CRAN if it is missing.
In **Applications → ShinyHarbor**, run **Install Shiny R package**, then **Start
ShinyHarbor**. **Run at login** creates a LaunchAgent for your account; **Stop
background server** unloads it. These commands show progress in Terminal.

Data stays in `~/Library/Application Support/ShinyHarbor`. The background process
runs as your user. Its log is `logs/service.log` inside that data directory.
**Uninstall ShinyHarbor** removes the installed files and retains user data.
The generated package is currently unsigned and unnotarized. Distribution signing
requires Apple Developer ID credentials; no Gatekeeper bypass is installed.

## Portable archives and the file-not-found fix

Extract the entire ZIP or tar archive into a writable directory. It includes
`harbor.toml` and `apps/hello/app.R`. Run `shinyharbor` from any working directory;
it finds the configuration alongside its executable. An installer writes a
`data-directory.txt` pointer when apps/settings live somewhere else.

For an older archive that lacks a configuration, create one explicitly:

```powershell
& "Z:\MoodysData\shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc\shinyharbor.exe" init "Z:\MoodysData\ShinyHarborData"
& "Z:\MoodysData\shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc\shinyharbor.exe" serve --config "Z:\MoodysData\ShinyHarborData\harbor.toml"
```

New builds also support `shinyharbor setup --directory PATH --install-shiny`.
Setup keeps existing settings. A bare new executable without any configuration
creates a per-user workspace. It does not download R or R packages implicitly.
`shinyharbor doctor` diagnoses missing R/Shiny, and `shinyharbor paths` reports
the resolved configuration, state and runtime locations without printing secrets.

Explicit `--config` takes priority, followed by `SHINYHARBOR_CONFIG`, working-directory
files, installer pointer, adjacent portable files, per-user configuration and
system configuration. Existing Unix `/etc/shiny-server/shiny-server.conf` remains
a fallback. Paths inside TOML are resolved relative to that file.

Apps: <http://127.0.0.1:3838/>. Dashboard: <http://127.0.0.1:3839/>.
Unlock the dashboard with `shinyharbor admin-token` from another terminal; use sudo
for Linux service state. Tokens are not placed in browser URLs or shortcuts.
