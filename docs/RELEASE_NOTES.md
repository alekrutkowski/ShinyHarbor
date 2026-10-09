# ShinyHarbor 0.1.0-alpha.1 — installers for evaluation

Start with the [step-by-step README](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.1/README.md).
It explains installation, your first app, R packages, the dashboard, HTTPS, updates
and troubleshooting for R users.

## Downloads in this release

| Computer | File to choose under Assets |
| --- | --- |
| Windows Intel/AMD 64-bit | `shinyharbor-0.1.0-alpha.1-x86_64-pc-windows-msvc-setup.exe` |
| Debian/Ubuntu Intel/AMD 64-bit | `shinyharbor_0.1.0~alpha.1-1_amd64.deb` |
| Fedora Intel/AMD 64-bit | `shinyharbor-0.1.0~alpha.1-1.x86_64.rpm` |
| Arch Linux Intel/AMD 64-bit | `shinyharbor-0.1.0_alpha.1-1-x86_64.pkg.tar.zst` |
| Portable Windows / Linux x64 | The `.zip` / `-linux-musl.tar.gz` archive |

The setup wizard lets Windows users choose program and data folders, including
another drive for app data. Portable archives now contain a configuration and
example app, and the executable finds them when launched from another directory.
This repairs the earlier bare-executable file-not-found startup problem.

R is installed separately. Windows setup can install Shiny into a private R
library; Debian/Ubuntu and Fedora packages declare R/Shiny dependencies. Arch
requires the documented additional Shiny installation step.

## Validation and limitations

These files were built and checked locally. Windows per-user installation,
upgrade and removal passed, preserving edited configuration and app data.
Both optimized executables passed seven first-run/real-Shiny checks. All three
Linux package formats passed checksum, metadata, hook-syntax, service-path,
license and extracted-executable checks. Shiny 1.14.0 was installed and loaded
from a private library using R 4.6.1.

**This is a partial-platform alpha prerelease.** Hosted CI is blocked by the
repository account's billing error, so the complete eight-platform matrix has
not run on this source. macOS and ARM installers are not included. Windows service
installation and actual Debian/Fedora/Arch package-manager lifecycles remain
unverified locally. Windows installers are unsigned. There is no hosted APT/DNF
repository or AUR entry.

Full Shiny Server compatibility and Professional feature parity are **not met**.
See [VALIDATION.md](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.1/VALIDATION.md)
and the [compatibility guide](https://github.com/alekrutkowski/ShinyHarbor/blob/v0.1.0-alpha.1/docs/COMPATIBILITY.md).
This publication does not claim a successful full CI run or stable-release certification.

Each artifact has a `.sha256` sidecar. `SHA256SUMS` lists the payloads actually
published, and `RELEASE_MANIFEST.json` records their hashes and source provenance.
The executables are unchanged from the validated installer build; the bundled
documentation has been refreshed. License notices and dependency information
are included. R, app-specific R packages, Posit Shiny Server and country databases
are not bundled.

Because the repository is private, downloads require a GitHub account with access.
