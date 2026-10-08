# Deployment

This guide describes the intended use of alpha source and provided templates, not
certified production deployment. First read SECURITY.md and COMPATIBILITY.md.

## Install a built artifact

A successful GitHub prerelease contains one archive per target, per-archive SHA-256
files and SHA256SUMS. Extract the archive matching the machine. The only gateway
runtime executable is `shinyharbor` or `shinyharbor.exe`; R and app packages are
separate. The embedded dashboard does not require Node. Keep license notices with
any redistributed archive.

`packaging/install.sh` and `packaging/install.ps1` install an already downloaded
archive after checking an explicitly supplied digest. They do not fetch a mutable
“latest” URL, run arbitrary remote shell code, elevate privileges, or edit PATH.
Digest verification is meaningful only when the expected digest is obtained from
a trusted release. There is no signed installer/MSI/package-manager distribution yet.

The source archive contains no prebuilt binaries. For a local build use `cargo build
--release` and a C compiler. CI-created `Cargo.lock` should be reviewed and committed
before reproducibility claims are made. glibc build/runtime floors depend on the
selected Ubuntu runner; test the actual oldest intended distribution. musl archives
avoid a glibc dependency but do not eliminate R's own dynamic library requirements.

## Linux service

Create a dedicated `shinyharbor` system user/group. Put read-only apps under
`/srv/shinyharbor/apps`, config at `/etc/shinyharbor/harbor.toml`, the executable at
`/usr/local/bin/shinyharbor`, and set `state_dir = "/var/lib/shinyharbor"` in config.
Install R and app packages where that service account can read them. Copy the unit:

```sh
sudo cp packaging/shinyharbor.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now shinyharbor
sudo journalctl -u shinyharbor -f
```

The unit uses a restricted account, private temporary directory, a read-only system
view, and a limited port-bind capability. Apps needing other writable paths require
explicit `ReadWritePaths` changes. It intentionally restricts access to home
directories, so home-installed R libraries may be unavailable. Do not weaken the
unit indiscriminately to make a particular app work. Put durable app output in an
explicitly permitted per-instance directory.

For native ACME, allow public TCP 443 and configure public DNS. The program does not
open the firewall. No root R interpreter should be needed. The private dashboard
can be reached using a tunnel:

```sh
ssh -L 3839:127.0.0.1:3839 your-user@your-server
```

Then open the local admin URL and supply the token. HTTPS for apps does not imply
that the separate dashboard listener is public or TLS-enabled.

## Windows

Install R and packages for the intended service account or a shared read-only R
library. Use an absolute TOML literal-string `rscript` path when a service cannot
find R on PATH. First run `check`, `doctor` and `serve` interactively.

The executable includes a native Service Control Manager entry point. The service
script defaults to the low-privilege LocalService account and protects an explicitly
specified state directory. It does not grant app/R/config access or open firewall
ports automatically.

```powershell
.\packaging\install-service.ps1 `
  -Binary 'C:\ShinyHarbor\shinyharbor.exe' `
  -Config 'C:\ShinyHarbor\harbor.toml' `
  -StateDirectory 'C:\ProgramData\ShinyHarbor'
# Review ACLs and ensure this directory matches state_dir, then:
Start-Service ShinyHarbor
```

`service` is an SCM entry point, not a substitute for `serve` in an ordinary console.
Startup status and failure paths still need native Windows acceptance testing. Review
and rotate `state_dir/service.log` separately. Windows Job Objects terminate child
process trees; they do not make apps mutually isolated or trusted.

## Containers

`packaging/Dockerfile` builds an unprivileged Linux image with R and current CRAN
Shiny. It is a convenience example, not a published image or a reproducible supply
chain. Pin base-image digests, dependency versions and lockfile, and retain all
transitive Rust/R/system dependency licenses before redistributing a derived image.
There is no GHCR multi-platform image publication workflow in this alpha.

Mount a config and app directory read-only, and a writable state volume. Configure
`listen = "0.0.0.0:3838"`, `allow_plaintext_public = true` only behind a trusted TLS
edge. Restrict who can reach that plaintext port. The admin listener remains
loopback **inside the container**; publishing its port does not make that loopback
listener externally reachable. Use an authenticated tunnel/exec arrangement, not
an accidental open administrative interface. For direct ACME, grant only the port
binding permission required for 443 and preserve the ACME cache volume.

## macOS and other platforms

The matrix provides Intel and ARM64 macOS binary jobs. Interactive deployment uses
the same CLI. No launchd installer or notarization is provided. Other architectures
and operating systems are outside the initial matrix and must not be advertised as
supported until their own build/runtime acceptance work exists.
