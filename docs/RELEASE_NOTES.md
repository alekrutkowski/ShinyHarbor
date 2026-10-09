# ShinyHarbor 0.1.0-alpha.2 — Sampling Lab and server administration

The [step-by-step README](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.2/README.md)
explains installation, your first app, administration, the terminal monitor and HTTPS.

## What changed

- **Sampling Lab** replaces the old example. Explore normal, skewed and heavy-tailed populations, Student-t confidence intervals, repeated-sampling coverage and Monte Carlo uncertainty. Experiments are reproducible and isolated between browser sessions. Download the sample and intervals as CSV, or a three-page PDF. Statistics, graphics and exports use base R; Shiny is the only additional R package.
- **Server administration** is available at `/admin` on the private dashboard port. Unlock it with the admin token. Common settings have labelled controls; the full configuration editor covers advanced settings and legacy `.conf` files. Inspect running values and the complete settings reference. Validate before saving, with private backups and detection of conflicting edits. Supported changes apply live; listener and other global changes require a restart, which the page explains.
- **Terminal monitor:** run `shinyharbor tui` for live status, worker/session counts and keyboard navigation. `--once` prints a readable snapshot. `shinyharbor open apps`, `open dashboard` and `open admin` follow your configured ports and HTTPS settings. Windows and macOS installer shortcuts use these commands.
- **HTTPS instructions** now distinguish trusted localhost certificates from automatically renewed public-domain ACME certificates. The README includes Windows, macOS, Debian/Ubuntu, Fedora, Arch and portable commands, service permissions, verification, renewal and restart steps.
- Upgrades refresh only the known, unmodified old example, keeping a backup. Customized apps, configuration and data are preserved.

## Downloads

| Computer | File under Assets |
| --- | --- |
| Windows Intel/AMD 64-bit | `shinyharbor-0.1.0-alpha.2-x86_64-pc-windows-msvc-setup.exe` |
| Debian/Ubuntu Intel/AMD 64-bit | `shinyharbor_0.1.0-alpha.2-1_amd64.deb` |
| Fedora Intel/AMD 64-bit | `shinyharbor-0.1.0-alpha.2-1.x86_64.rpm` |
| Arch Linux Intel/AMD 64-bit | `shinyharbor-0.1.0_alpha.2-1-x86_64.pkg.tar.zst` |
| Portable Windows / Linux x64 | The `.zip` / `-linux-musl.tar.gz` archive |

R is installed separately. Windows setup can install Shiny into a private R
library; Debian/Ubuntu and Fedora packages declare R/Shiny dependencies. Arch
requires the documented additional Shiny installation step.

## Validation and limits

Optimized Windows x64 and Linux x64 executables passed administration API,
configuration preservation, example upgrade, real-Shiny startup and verified
HTTPS checks. Windows browser checks cover the new administration UI and Sampling
Lab, including mobile layouts, session isolation and actual downloads. Base-R
checks compare intervals with `t.test` and verify coverage, density normalization
and random-state isolation. Linux PTY checks exercise terminal keys, restoration
and server survival. See [VALIDATION.md](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.2/VALIDATION.md)
for version-specific evidence and remaining platform checks.

**This is a partial-platform alpha prerelease.** Hosted CI has been blocked by the
repository account's billing error. The complete eight-platform matrix has not
passed on this source. macOS and ARM installers are not included. Windows service
acceptance and actual Debian/Fedora/Arch package-manager lifecycles remain
unverified locally. Installers are unsigned. There is no hosted APT/DNF repository
or AUR entry. macOS installer recipes and CI jobs are supplied, not certified.

Local PEM certificates must be renewed externally; automatic renewal applies to
public-domain ACME mode. Administration remains on its separate loopback HTTP
listener; use an SSH tunnel for remote access. Saving settings does not install
R packages, alter browser trust, open a firewall or restart the server.

Full Shiny Server compatibility and Professional feature parity remain **unmet**;
see the [compatibility guide](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.2/docs/COMPATIBILITY.md).
This release does not claim stable-release certification or a successful full CI run.

Every payload has a `.sha256` sidecar. `SHA256SUMS` and `RELEASE_MANIFEST.json`
record the published files, hashes, source provenance and validation scope.
Dependency notices and the resolved dependency inventory are included. R,
app-specific R packages and country databases are not bundled.

Because the repository is private, downloads require a GitHub account with access.
