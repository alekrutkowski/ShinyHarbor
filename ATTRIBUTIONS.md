# Attributions and acknowledgments

Original ShinyHarbor code is MIT licensed, copyright Alek Rutkowski,
<alek.rutkowski@gmail.com>. No ownership of dependencies or external runtimes is
claimed. No Posit Shiny Server source code or binaries are copied into this project.
The original R launcher and dashboard are part of ShinyHarbor.

## Rust dependencies

Thanks to the authors and contributors of every crate listed below and their
transitive dependencies. Exact resolved versions, authors, upstream repositories,
license expressions and retained license texts are generated during packaging from
Cargo metadata. This source-level list does not replace those legal notices.

- anyhow: error context and propagation.
- argon2: password hashing/verification.
- axum, axum-server: request routing, HTTP serving and TLS integration.
- base64, hex: token/identifier encodings.
- bytes, http, http-body, hyper, hyper-util: HTTP types, buffers and streaming transport.
- clap: command-line parsing/help.
- fs2: state-directory process locking.
- futures-util: asynchronous stream utilities.
- hmac, sha2, subtle: keyed signatures, hashing and constant-time comparison primitives.
- ipnet: explicit trusted-proxy network ranges.
- maxminddb: local country database reading.
- nix: Unix account lookup, supplementary groups and process identity.
- process-wrap: portable child lifecycle, Unix process groups and Windows Job Objects.
- rand: operating-system randomness.
- rpassword: hidden terminal password input.
- rusqlite and bundled SQLite: local aggregated analytics storage.
- rustls, rustls-acme and transitive cryptographic dependencies: TLS and ACME.
- serde, serde_json, toml: configuration and structured data.
- tokio, tokio-util: asynchronous execution, cancellation and task tracking.
- tokio-tungstenite: native application WebSocket connections.
- tempfile: private temporary runtime directories and test fixtures.
- tracing, tracing-subscriber: structured operational logging.
- url: origin/URL parsing.
- windows-service: Service Control Manager integration.
- http-body-util, tower: HTTP body and service utilities.

The packaging tool retains all resolved crates conservatively, including build/dev
packages, and stops for missing license texts. A Rust library's license does not
relicense bundled C libraries or other nested material; retained upstream notices
must be reviewed before redistribution. No fake pre-resolved dependency list is supplied.

## External runtimes and data

The embedded SockJS browser client is sockjs-client 1.6.1, distributed under MIT.
Its retained notice is `assets/vendor/sockjs-LICENSE`; release packages include it
in `licenses/sockjs-client-1.6.1/LICENSE` and list the library in the SBOM.
Source: https://github.com/sockjs/sockjs-client/tree/v1.6.1.
The transport adapter, replay protocol, and multiplex runtime are original
ShinyHarbor implementations; upstream server code is not included.

R, R packages and their system libraries are installed separately and retain their
own licenses. R is GPL-licensed; CRAN Shiny is distributed under MIT plus its license
file. Posit's Shiny Server OSS is AGPL-licensed and is an optional separately installed
upstream, not relicensed under this project's MIT license. Professional software
requires whatever license its publisher specifies. Review combined-distribution
obligations for your deployment; this text is not legal advice.

Country databases are not included. Obtain and use a compatible MMDB file under its
provider's current terms. No geographic data or external fonts are bundled. The
dashboard uses system fonts and original HTML/CSS/JavaScript rather than copied
component/template source.

## Development and distribution tooling

GitHub Actions maintained by GitHub (`checkout`, `upload-artifact`, `download-artifact`,
`setup-python`, `setup-node`) and the R community (`r-lib/actions/setup-r`) provide CI.
Python tests use aiohttp, yarl (transitively), Playwright and PyYAML. Chromium and
Playwright browser binaries retain their own licenses and are test tools, not server
runtime dependencies. Rustup, Cargo, rustfmt, Clippy and platform C compilers are
build tools. Docker example base images, R libraries and operating-system packages
have independent licensing and notice requirements.

Shiny, Shiny Server, Posit, Rust, GitHub and other product names identify their
respective projects. ShinyHarbor is not an official Posit product and no affiliation
or endorsement is asserted.
