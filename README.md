# ShinyHarbor

**Run your R Shiny apps on a computer that other people can visit in their web browser.**

ShinyHarbor starts your R apps, serves their pages, and provides a private usage
dashboard. It can also obtain and renew HTTPS certificates. You keep writing
ordinary Shiny code; your visitors only need a browser.

**Current version: 0.1.0-alpha.1, for evaluation.** This is **not a certified drop-in replacement**
for Posit's Shiny Server, and full Shiny Server Professional compatibility is not
complete. Windows x64 and Linux x64 have local validation; macOS and ARM installer
builds are pending. See [what has actually been tested](VALIDATION.md).

## Start here

1. [Choose your download](#choose-your-download).
2. Install on [Windows](#windows), [Debian or Ubuntu](#debian-and-ubuntu),
   [Fedora](#fedora), [Arch Linux](#arch-linux), or [macOS](#macos).
3. [Try the example, then add your own app](#add-your-own-shiny-app).
4. [Open the usage dashboard](#open-the-usage-dashboard).
5. When you are ready, [share apps with other people](#share-apps-with-other-people).

Also: [other Linux distributions](#other-linux-distributions) ·
[portable ZIP/tar downloads](#portable-downloads) ·
[updates and removal](#updates-backups-and-removal) ·
[troubleshooting](#troubleshooting) · [developer information](#for-developers).

## What you need

- **One computer to run the apps.** It can be your own computer for a first test,
  or a dedicated machine for longer-running use. It must stay on while people use
  your apps. This is the *server*.
- **R on that computer**, plus `shiny` and the R packages your apps use. RStudio is
  optional. The installers below explain how to get these dependencies.
- **An app folder** containing `app.R`, or `ui.R` and `server.R`, together with the
  files the app needs. The installer includes a small example to get you started.

The installer contains the ready-to-run ShinyHarbor program. You do not need to
compile Rust or install Cargo to use it.

ShinyHarbor initially accepts connections from **the server computer itself**.
The address `127.0.0.1` means “this computer.” Getting the example working locally
is the first step; making it available to other computers is a separate step below.

## Choose your download

Open [**GitHub Releases**](https://github.com/alekrutkowski/ShinyHarbor/releases/tag/v0.1.0-alpha.1)
and expand **Assets**. Choose an installer from this table. GitHub's automatically
generated **Source code** downloads contain source files, not an installer.

| Your computer | Download for this alpha | Availability |
| --- | --- | --- |
| Windows on Intel/AMD, 64-bit | [Windows setup `.exe`][windows-setup] | Available; install, upgrade and uninstall tested |
| Debian or Ubuntu, Intel/AMD 64-bit | [Debian/Ubuntu `.deb`][linux-deb] | Available; package and executable checked |
| Fedora, Intel/AMD 64-bit | [Fedora `.rpm`][linux-rpm] | Available; package and executable checked |
| Arch Linux, Intel/AMD 64-bit | [Arch `.pkg.tar.zst`][linux-arch] | Available; package and executable checked |
| macOS, Intel | `*-x86_64-apple-darwin.pkg` | Download pending |
| macOS, Apple Silicon (M-series) | `*-aarch64-apple-darwin.pkg` | Download pending |
| Windows ARM64 | `*-aarch64-pc-windows-msvc-setup.exe` | Download pending |
| Linux ARM64, including Arch Linux ARM | `.deb` with `arm64`, or `.rpm` / `.pkg.tar.zst` with `aarch64` | Downloads pending |

`x64`, `x86_64` and `amd64` mean the usual 64-bit Intel/AMD architecture. `ARM64`
and `aarch64` mean the 64-bit ARM architecture. On Windows, look at **Settings →
System → About → System type**. On a Mac, **Apple menu → About This Mac** shows an
Intel processor or an Apple chip. On Linux, `uname -m` prints the architecture.

Only the available files are attached to this prerelease. GitHub's account billing
block has prevented the remaining build jobs from starting. The Linux packages
have been inspected and their executables run, but a full installation on each
distribution has not yet been tested. The pending platforms have installer recipes
and release jobs prepared, not validated downloads.

The repository is currently private: sign in to a GitHub account with repository
access to download the files. Installing a local DEB/RPM/Arch file does not add a
ShinyHarbor software repository or enable automatic updates.

## Windows

### Install and open the example

1. Install [R for Windows from CRAN](https://cran.r-project.org/bin/windows/base/)
   if it is not already installed. ShinyHarbor finds standard R installations
   without requiring you to edit PATH.
2. Download and double-click the [Windows setup installer][windows-setup].
3. Choose installation **for your account** for a first test. Keep the suggested
   program folder, or select another one.
4. On the **Application data** page, choose where to keep your apps, settings,
   R packages and usage data. You can choose another drive, for example
   `Z:\ShinyHarborData`, if C: is short of space. Keep this folder for future upgrades.
5. Select **Install or update the Shiny R package**. This needs an Internet connection
   and installs Shiny into the selected data folder. Leave the background service
   option off for your first test.
6. Finish setup and open **Start menu → ShinyHarbor → Start ShinyHarbor**.
   Keep its console window open.
7. Open <http://127.0.0.1:3838/hello/> in your browser. You should see the example app.

The alpha installer is unsigned, so Windows may identify an unknown publisher.
Use the release checksums and your organisation's software-installation policy.

The Start menu also has **Application folders**, **Edit configuration**, and
**Usage dashboard** shortcuts. **Application folders** opens the exact place
where you will copy your own apps.

### Keep it running in the background

For a machine that should serve apps without an open console, run setup in
**all-users** mode and select **Run in the background as a Windows service**.
This requires administrator rights. A *service* is a program Windows starts and
keeps running in the background, including after a restart.

The service option installs its own Shiny library. Use local disks for the program
and data folders: a mapped/network Z: drive is available to your login session,
but generally not to the service. Use the ordinary Start shortcut for a network
drive. Service installation is implemented but has not yet passed local acceptance testing.

To stop or restart the service, open the Windows **Services** application, find
**ShinyHarbor**, and choose **Stop** or **Restart**. Stop a foreground copy before
starting the service so that they do not compete for the same port.

### Run diagnostic commands

Open **PowerShell**, not the R console. For the default installation for your account:

```powershell
$Harbor = "$env:LOCALAPPDATA\Programs\ShinyHarbor\shinyharbor.exe"
& $Harbor paths
& $Harbor doctor
```

If you chose another program folder, replace the first line with its full path,
for example `$Harbor = 'Z:\ShinyHarbor\shinyharbor.exe'`. An all-users installation
normally uses `C:\Program Files\ShinyHarbor\shinyharbor.exe`.
`paths` shows where your settings and R library are; `doctor` checks R and Shiny.
The installer does not add the program to PATH, so use its full path as above.

## Debian and Ubuntu

Download the [`.deb` package][linux-deb]. Open a **Terminal in the folder containing
the downloaded file**. For the current Intel/AMD 64-bit package, run:

```sh
sudo apt update
sudo apt install ./shinyharbor_0.1.0-alpha.1-1_amd64.deb
```

`sudo` asks for your Linux account password to allow system installation. The
package manager installs R and its packaged Shiny dependency too. Ubuntu systems
need access to the **Universe** package repository for `r-cran-shiny`; if APT cannot
find it, follow [Ubuntu's repository instructions](https://help.ubuntu.com/community/Repositories/Ubuntu).

ShinyHarbor installs a background service. Check it and open the example:

```sh
systemctl status shinyharbor --no-pager
sudo -u shinyharbor shinyharbor doctor --config /etc/shinyharbor/harbor.toml
```

On the same computer, visit <http://127.0.0.1:3838/hello/>.
Ubuntu-based distributions such as Linux Mint and Pop!_OS may use the DEB when
their repositories provide the listed dependencies; they are not separately
validated. Continue with [Linux folders and service controls](#linux-folders-and-service-controls).

## Fedora

Download the [`.rpm` package][linux-rpm]. Open a **Terminal in its download folder**:

```sh
sudo dnf install ./shinyharbor-0.1.0-alpha.1-1.x86_64.rpm
systemctl status shinyharbor --no-pager
sudo -u shinyharbor shinyharbor doctor --config /etc/shinyharbor/harbor.toml
```

DNF installs R and Fedora's `R-shiny` package. Open
<http://127.0.0.1:3838/hello/> on the server computer.
Continue with [Linux folders and service controls](#linux-folders-and-service-controls).

An RPM file is not automatically compatible with every RPM-based distribution.
For RHEL, Rocky Linux, AlmaLinux or openSUSE, use the
[other Linux guidance](#other-linux-distributions) until native packages for those
systems have been validated.

## Arch Linux

Download the [Arch package][linux-arch]. Open a **Terminal in its download folder**:

```sh
sudo pacman -Syu --needed r base-devel
sudo pacman -U ./shinyharbor-0.1.0_alpha.1-1-x86_64.pkg.tar.zst
```

The first command also updates the system's packages, following
[Arch's full-upgrade guidance](https://wiki.archlinux.org/title/System_maintenance#Partial_upgrades_are_unsupported).
Arch supplies R; install Shiny separately into the server's R library:

```sh
sudo -u shinyharbor shinyharbor setup --directory /etc/shinyharbor --install-shiny
sudo systemctl restart shinyharbor
sudo -u shinyharbor shinyharbor doctor --config /etc/shinyharbor/harbor.toml
```

The Shiny download may take several minutes because R packages can be compiled
from source. Open <http://127.0.0.1:3838/hello/> on the server computer.
There is currently no ShinyHarbor AUR entry. Arch Linux ARM needs a separate
`aarch64` build; do not install the `x86_64` package on it.

## Linux folders and service controls

These paths apply to the DEB, RPM and Arch installers:

| What you want to find | Where it is |
| --- | --- |
| Your app folders | `/srv/shinyharbor/apps/` |
| Settings | `/etc/shinyharbor/harbor.toml` |
| R packages used by the server | `/var/lib/shinyharbor/R/library/` |
| Usage database, keys and saved state | `/var/lib/shinyharbor/` |

Use these commands in a Terminal:

```sh
sudo systemctl restart shinyharbor     # After changing server settings
sudo systemctl stop shinyharbor        # Stop serving apps
sudo systemctl start shinyharbor       # Start again
sudo journalctl -u shinyharbor -n 50 --no-pager  # Recent service messages
```

The packages use **systemd**, Linux's background-service manager. A container or
minimal distribution without systemd should use [portable deployment](#portable-downloads)
or the [container instructions](docs/DEPLOYMENT.md#containers).

The packages install the distribution's R/Shiny versions, which can be older than
CRAN. If your app needs current CRAN Shiny, first install R package build tools
(`r-base-dev` and `libuv1-dev` on Debian/Ubuntu; `R-devel`, `gcc-c++`, `make` and
`libuv-devel` on Fedora), then run the same `setup --install-shiny` command shown
for Arch. This changes the private ShinyHarbor library; it does not upgrade R itself.

## macOS

**The macOS installers are not attached to this alpha yet.** The following steps
describe the prepared installer; use them when a matching PKG appears in Releases.

1. In **Apple menu → About This Mac**, check whether you have Apple Silicon or Intel.
2. Install the matching [R for macOS](https://cran.r-project.org/bin/macosx/) from CRAN.
   Check CRAN's macOS version requirements for the R download you select.
3. Download the corresponding ShinyHarbor `.pkg` and double-click it.
4. Open **Applications → ShinyHarbor → Install Shiny R package** and let it finish.
5. Open **Start ShinyHarbor**, then visit <http://127.0.0.1:3838/hello/>.
   Keep the Terminal window open while using the foreground server.

Your data folder is `~/Library/Application Support/ShinyHarbor`. In Finder, use
**Go → Go to Folder** and paste that path. `~` means your home folder.
Put apps inside its `apps` subfolder.

**Run at login** sets up a background process for your account. Close the foreground
server first. **Stop background server** stops it and removes the login startup
entry. This is a login service, so it starts when you log in, not before.

The program is installed at `/usr/local/bin/shinyharbor`. In a new Terminal:

```sh
/usr/local/bin/shinyharbor paths
/usr/local/bin/shinyharbor doctor
```

The prepared PKG is unsigned and unnotarized; follow your Mac's software-installation
policy. Signing, notarization and native macOS installer testing are still pending.

## Other Linux distributions

For openSUSE, RHEL-family systems, Alpine, and systems without systemd, the portable
Linux archive is the evaluation route. These distributions are **not separately
validated**, and there are no native ShinyHarbor packages for them in this release.

1. Install R using the instructions for your distribution in
   [CRAN's Linux installation guides](https://cran.r-project.org/bin/linux/) or its
   distribution package manager. Install its R development/build tools if Shiny
   packages need compilation.
2. Download the [Linux x64 portable archive][linux-portable] for Intel/AMD 64-bit.
   It uses a static *musl* build: the server executable does not depend on a
   particular glibc version. R and its packages still need their own system libraries.
3. Follow the portable instructions below. Start it in the foreground first.
4. For unattended operation, use your system's service manager. The supplied
   [systemd service template](packaging/shinyharbor.service) is an example only for
   systems using systemd; follow [manual service setup](docs/DEPLOYMENT.md#linux-service).

There is no 32-bit release and no FreeBSD or other non-Linux Unix release in the
current build matrix.

## Portable downloads

Use this option to try the program without a system installer, or to keep it on
another drive. Download the [Windows ZIP][windows-portable] or
[Linux x64 tar archive][linux-portable] and extract **the entire archive** into a
writable folder. Keep the files together: the archive includes `harbor.toml`,
`apps/hello/app.R`, documentation and license notices.

On Windows, right-click the ZIP and select **Extract All**. On Linux, you can
extract from a Terminal in your download folder:

```sh
tar -xzf shinyharbor-0.1.0-alpha.1-x86_64-unknown-linux-musl.tar.gz
cd shinyharbor-0.1.0-alpha.1-x86_64-unknown-linux-musl
```

On Windows, open PowerShell in the extracted folder:

```powershell
.\shinyharbor.exe setup --directory . --install-shiny
.\shinyharbor.exe doctor --config .\harbor.toml
.\shinyharbor.exe serve --config .\harbor.toml
```

On Linux, open a Terminal in the extracted folder:

```sh
./shinyharbor setup --directory . --install-shiny
./shinyharbor doctor --config ./harbor.toml
./shinyharbor serve --config ./harbor.toml
```

Install R first. `setup` installs Shiny when requested and keeps existing settings.
The server stays in this terminal; **Ctrl+C** stops it. Open
<http://127.0.0.1:3838/hello/> to try it. A bare executable also finds the settings
beside it, even when launched from another working directory.

## Add your own Shiny app

Start with an app that works in a fresh R session with `shiny::runApp("my-app")`.
Its code must load its own packages and data; objects left in your RStudio workspace
are not available to a new server R process.

1. Name the app folder something simple, such as `my-app`.
2. Copy the **whole folder**, including data files and `www` assets, into the apps
   folder for your installation. On Windows, use the **Application folders** shortcut.
3. The result should look like this, without an extra folder level:

   ```text
   apps/
     hello/
       app.R
     my-app/
       app.R
       www/
       data.csv
   ```

4. Install any additional R packages into a library the server can read; see below.
5. Visit <http://127.0.0.1:3838/my-app/>. The folder name becomes part of the address.
   The first visit may take longer while R starts. Newly added apps do not require
   a server restart.

For a Linux package installation, copy from the folder containing `my-app`:

```sh
sudo cp -R ./my-app /srv/shinyharbor/apps/
sudo chown -R root:shinyharbor /srv/shinyharbor/apps/my-app
sudo chmod -R u=rwX,g=rX,o= /srv/shinyharbor/apps/my-app
```

The last two commands make the copied app readable by the service while keeping
editing under administrator control. Apps that write files need a separate writable
location; see [deployment permissions](docs/DEPLOYMENT.md#linux-service).

### Install the R packages your app uses

ShinyHarbor does not infer your app's dependencies or copy your personal R library.
For example, if your app uses `ggplot2` and `dplyr`, install those for the server too.

On Windows or macOS, open a **fresh R session**. Set `lib` to the **R library** path
shown by `shinyharbor paths`, then run R code like this. The path below is an example;
replace it with yours. Forward slashes also work in Windows R paths.

```r
lib <- "Z:/ShinyHarborData/R/library"
dir.create(lib, recursive = TRUE, showWarnings = FALSE)
.libPaths(lib, include.site = FALSE)
install.packages(c("ggplot2", "dplyr"), lib = lib,
                 repos = "https://cloud.r-project.org")
```

Use an account allowed to write that library. For the Linux packages, run this
in a **Terminal**, replacing the package names as appropriate:

```sh
sudo -u shinyharbor env R_LIBS=/var/lib/shinyharbor/R/library R_LIBS_USER=/var/lib/shinyharbor/R/library R_LIBS_SITE= Rscript --vanilla -e 'install.packages(c("ggplot2", "dplyr"), lib="/var/lib/shinyharbor/R/library", repos="https://cloud.r-project.org")'
```

Some R packages need additional system libraries or compilers. Use the package's
installation instructions if R reports missing system dependencies. If your app
uses `renv`, restore its environment on the server using the same R version and
an account with access to that project; copying a library from another OS is not sufficient.

To apply edits to an existing app, create or update an empty `restart.txt` inside
its app folder. New visits start the updated app while existing sessions finish.
Restarting the whole server also applies edits, but interrupts active sessions.

## Open the usage dashboard

On the server computer, open <http://127.0.0.1:3839/> or use the Windows
**Usage dashboard** shortcut. The dashboard needs an *admin token*: a secret
that proves you administer this server.

Get it in a second terminal while the server is running:

| Installation | Command |
| --- | --- |
| Windows | `& $Harbor admin-token` after setting `$Harbor` as in the Windows section; use administrator PowerShell for an all-users service |
| Linux DEB/RPM/Arch | `sudo shinyharbor admin-token --config /etc/shinyharbor/harbor.toml` |
| macOS | `/usr/local/bin/shinyharbor admin-token` |
| Portable | `./shinyharbor admin-token --config ./harbor.toml`; on Windows use `.\shinyharbor.exe` instead |

Paste the token into the dashboard's unlock field. Keep it private; it is not
an app-user password and should not be included in a shared link.

![Example dashboard with synthetic demonstration data](docs/images/dashboard-preview.png)

The dashboard shows app activity, traffic over time, connections and referring
sites. Country estimates require a separate country database; they are not
available automatically. See [analytics settings](docs/CONFIGURATION.md#analytics).
“Visitor-days” are daily estimates, not a count of unique people; reconnects can
increase connection counts. The analytics database does not store raw IP addresses,
but app code and other logs may record their own data. Analytics can be disabled.

## Share apps with other people

An address beginning `127.0.0.1` only works on the computer where it is opened.
Sending that link to a colleague does not let them reach your server.

### Try a remote server privately

If you have SSH access to a Linux/macOS server, run this on **your own computer**,
replacing the account and server name:

```sh
ssh -L 3838:127.0.0.1:3838 -L 3839:127.0.0.1:3839 your-user@your-server
```

Keep that connection open. You can then use the example and dashboard addresses
above in your own browser. SSH carries the traffic securely; the server's default
local-only settings can stay in place. If you already run a local ShinyHarbor,
stop it first or use different local ports for the tunnel.

### Publish at your own HTTPS address

For an address such as `https://shiny.example.org/my-app/`, you need a domain name
pointing to the server and permission to accept incoming connections. If your
organisation manages domains or firewalls, give these steps to its IT administrator.

1. Point the domain's public DNS record to the server. Allow incoming **TCP 443**
   for HTTPS and **TCP 80** for the HTTP-to-HTTPS redirect. Forward those ports
   through a router if one sits in front of the server.
2. Open `harbor.toml`: use **Edit configuration** on Windows, the data folder on
   macOS/portable installs, or `/etc/shinyharbor/harbor.toml` on packaged Linux.
3. Change the existing top-level listener lines, near the beginning of the file:

   ```toml
   listen = "0.0.0.0:443"
   admin_listen = "127.0.0.1:3839"
   ```

4. Replace the existing `[tls]` block, or add this block before `[[sites]]` if there
   is no TLS block. Keep your existing app, state and R-library paths.

   ```toml
   [tls]
   mode = "acme"
   domains = ["shiny.example.org"]
   email = "you@example.org"
   production = false
   ```

5. Replace the domain and email with your real values, validate with
   `shinyharbor check --config /path/to/harbor.toml`, and restart the server using
   your platform's instructions above. On Windows, use `& $Harbor check --config`
   followed by the full config path in quotes.
6. `production = false` uses Let's Encrypt's **test** service. Its certificates
   are not trusted by browsers. After issuance succeeds in the server logs,
   change it to `production = true` and restart to request a trusted certificate.
7. Visit `https://your-real-domain/my-app/` and share that address.

The Linux package's service can bind the required ports without running R as root.
Other Unix foreground installations may need an administrator to configure low-port
access or an existing reverse proxy; see [deployment](docs/DEPLOYMENT.md).
Certificate validation requires public port 443 to reach ShinyHarbor directly;
an HTTPS-terminating proxy/CDN cannot intercept that challenge. Wildcard domains
are not supported by this certificate setup. HTTPS configuration is implemented,
but public-CA production deployment has not been validated here.

ShinyHarbor renews certificates while it is running. Back up and retain its state
directory, including `acme`, so account keys and certificates survive upgrades.
The dashboard remains private; use the SSH tunnel for remote administration.

**HTTPS encrypts traffic; it does not decide who may use an app.** Apps are
accessible without a login by default. For private apps, configure
[password protection](docs/CONFIGURATION.md#password-protection) or
[your organisation's sign-in proxy](docs/CONFIGURATION.md#authentication-proxies-and-groups)
before exposing them. ShinyHarbor does not open your firewall automatically.

## Updates, backups and removal

Before updating, back up your app folders, `harbor.toml`, private R library and
state directory. For a consistent copy of the usage database, stop the server
while taking the backup. Keep state backups private because they contain keys.

| Platform | Update | Remove the program |
| --- | --- | --- |
| Windows | Run the newer setup installer, keeping the same folders | Settings → Apps → ShinyHarbor → Uninstall |
| Debian/Ubuntu | `sudo apt install ./NEW-PACKAGE.deb` | `sudo apt remove shinyharbor` |
| Fedora | `sudo dnf upgrade ./NEW-PACKAGE.rpm` | `sudo dnf remove shinyharbor` |
| Arch | `sudo pacman -U ./NEW-PACKAGE.pkg.tar.zst` | `sudo pacman -R shinyharbor` |
| macOS, when available | Run the newer PKG | Applications → ShinyHarbor → Uninstall ShinyHarbor |
| Portable | Stop the server; unpack the new release into a separate folder and point it at your existing config | Remove the program folder only after separating/backing up your app data |

Replace `NEW-PACKAGE` with the actual downloaded filename. Native installers keep
your apps and settings during upgrades, and retain data when the program is removed.
Linux package-manager *purge* can remove configuration. Portable archives contain
a sample configuration: do not overwrite your working `harbor.toml` with it.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| GitHub download says “404” | Sign in with an account that has access to this private repository. Check that the table lists the file as available. |
| `shinyharbor` is not recognised | On Windows use the Start menu shortcut or the full executable path with `&` in PowerShell. For an extracted archive use `./shinyharbor` or `.\shinyharbor.exe`. |
| “Cannot find configuration” / old “os error 2” | Extract the entire current archive or use the installer. For a standalone binary, run `setup --directory "YOUR-DATA-FOLDER"`, then `serve --config "YOUR-DATA-FOLDER/harbor.toml"`. Very old builds use `init` instead of `setup`. |
| “Cannot start R” | Install R on the server and run `doctor`. If R is in a custom location, set `rscript` in `harbor.toml`; see [configuration](docs/CONFIGURATION.md). |
| R says there is no package called `shiny` | Select the install-Shiny option, or run `setup --directory "YOUR-DATA-FOLDER" --install-shiny`. For a Linux package use `/etc/shinyharbor` as that directory and run as the `shinyharbor` account. |
| Your app cannot find another package | Install it into the server's R library. A Windows service or Linux service does not use your RStudio account's personal library. |
| The example works, but your app does not | Run the app in a fresh R session, check its data paths, and inspect its worker log in the state directory's `logs` subfolder. Use paths relative to the app folder instead of your own desktop. |
| “Address already in use” | Check whether another ShinyHarbor or Shiny Server is already running. Stop the duplicate, or select unused app/dashboard ports in `harbor.toml`. |
| A colleague cannot open your link | `127.0.0.1` is local to each computer. Follow the sharing instructions; check the server is on and its domain/firewall settings are correct. |
| Browser certificate warning with test HTTPS | `production = false` creates a test certificate. Complete the HTTPS steps and switch to production; do not send users the staging-certificate endpoint. |

`check` validates configuration; `doctor` checks R and Shiny. Neither command
checks every package your app needs. For further help, include your OS, the result
of `shinyharbor --version`, the relevant error, and what command you ran. Remove
admin tokens, passwords and private app data before sharing logs.

<details>
<summary>Optional: verify a downloaded file's checksum</summary>

A checksum detects a damaged or changed download. Download the matching `.sha256`
file from the same release and compare its long hexadecimal value with your result.
It does not replace a trusted publisher signature.

Windows PowerShell, using the actual downloaded filename:

```powershell
Get-FileHash .\shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc-setup.exe -Algorithm SHA256
```

Linux, with the installer and its sidecar in the same directory:

```sh
sha256sum -c shinyharbor_0.1.0-alpha.1-1_amd64.deb.sha256
```

On macOS, use `shasum -a 256 FILE` and compare the value. `SHA256SUMS` on the release
lists only the files actually published in that release.

</details>

## Existing Posit Shiny Server installations

Keep your existing service intact while evaluating ShinyHarbor. It understands a
documented subset of `shiny-server.conf`, but unsupported directives fail rather
than being silently ignored, and runtime/Professional parity remains incomplete.
For a test, copy the configuration, use different listening ports and a separate
state directory, then run `shinyharbor check --config COPY.conf` before starting it.
See the [compatibility guide](docs/COMPATIBILITY.md) for the outstanding differences.

ShinyHarbor can also forward requests to an independently running Shiny Server.
That mode still depends on the original server and its limitations; see
[upstream configuration](docs/CONFIGURATION.md#upstream).

## For developers

The server is written in Rust, using Tokio and axum/hyper for HTTP, rustls/ACME for
TLS, and SQLite for usage summaries. R processes start on demand, can use
configurable per-app pools, and stop when idle. Increasing worker limits can run
more R processes; it does not make a single R computation faster or make app code
safe to run from untrusted authors.

The build matrix targets Windows, macOS, and Linux (GNU/musl), each on x64 and ARM64.
Local tests used R 4.6.1 and Shiny 1.14.0. The exact executed tests, limitations and
pending platforms are recorded in [VALIDATION.md](VALIDATION.md). Full compatibility
and stable releases remain gated by the [conformance inventory](conformance/required.json).

To build from source, install current stable Rust and your platform's C/C++ build
tools (Microsoft C++ Build Tools on Windows), then run:

```sh
cargo build --locked --release
```

The executable is `target/release/shinyharbor`, or `target\release\shinyharbor.exe`
on Windows. Install R separately and follow the portable setup instructions using
that executable. Building from source can consume several gigabytes of disk space.

Further documentation:

- [Installation reference](docs/INSTALLATION.md), including silent setup and custom paths.
- [Configuration reference](docs/CONFIGURATION.md), including worker pools, authentication and analytics.
- [Deployment](docs/DEPLOYMENT.md) and [operations](docs/OPERATIONS.md).
- [Compatibility](docs/COMPATIBILITY.md), [security](SECURITY.md), and [validation](VALIDATION.md).
- [CI and release process](docs/RELEASING.md).

License: MIT, copyright Alek Rutkowski. Bundled dependency notices accompany the
downloads; external-runtime acknowledgments are in [ATTRIBUTIONS.md](ATTRIBUTIONS.md).

[windows-setup]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc-setup.exe
[linux-deb]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor_0.1.0-alpha.1-1_amd64.deb
[linux-rpm]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor-0.1.0-alpha.1-1.x86_64.rpm
[linux-arch]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor-0.1.0_alpha.1-1-x86_64.pkg.tar.zst
[windows-portable]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc.zip
[linux-portable]: https://github.com/alekrutkowski/ShinyHarbor/releases/download/v0.1.0-alpha.1/shinyharbor-0.1.0-alpha.1-x86_64-unknown-linux-musl.tar.gz
