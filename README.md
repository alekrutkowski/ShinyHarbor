# ShinyHarbor

A Rust gateway and native process host for R Shiny applications, with integrated
TLS/ACME, configurable worker pools, and a private usage dashboard.

**Status: 0.1.0-alpha.1. This is source code for evaluation, not a certified drop-in replacement
for Posit's Shiny Server, and not a production-ready equivalent of Shiny Server Professional.**
The requested zero-compatibility-gap acceptance criterion is **not met**.
The complete upstream configuration/runtime contract and full Professional feature
parity remain open. See [COMPATIBILITY](docs/COMPATIBILITY.md) before deploying.
Stable releases are deliberately blocked while these requirements remain unresolved.

Rust builds, unit tests and live gateway checks have passed on Windows x64 and
Linux x64. Real Shiny 1.14.0 browser tests cover seven transports, reconnect,
multi-worker tabs, bookmarking, proxy users/groups, interactive R Markdown,
resource limits, configuration reloads, and the live dashboard. Reconnect wire checks also
passed against Posit Shiny Server 1.5.23.1030. Local Pebble tests exercise actual
ACME issuance, restart persistence, automatic renewal and live PEM replacement
with TLS verification. Gzip/deflate negotiation is covered by live HTTP checks.
Custom templates, worker log retention, access logs and connection inactivity
deadlines also have executable tests. Independent TLS listeners, larger Shiny
messages, Unix login profiles and worker supervisors have live checks too.
Five native CI build/package jobs passed on the initial commit; the complete
current-source matrix remains unverified. See [VALIDATION](VALIDATION.md) for scope.

## Start locally

Install R and the CRAN `shiny` package for the account that will run the service.
Build with a current stable Rust toolchain, including a platform C compiler for
bundled SQLite and cryptography. Windows requires the Microsoft C++ build tools.

```sh
cargo build --release
# Linux:
./target/release/shinyharbor init demo
./target/release/shinyharbor doctor --config demo/harbor.toml
./target/release/shinyharbor serve --config demo/harbor.toml
```

```powershell
# Windows, after cargo build --release:
.\target\release\shinyharbor.exe init demo
.\target\release\shinyharbor.exe doctor --config demo\harbor.toml
.\target\release\shinyharbor.exe serve --config demo\harbor.toml
```

The example app is at `http://127.0.0.1:3838/hello/`. The local dashboard is at
`http://127.0.0.1:3839/`. In another terminal, run `shinyharbor admin-token --config
demo/harbor.toml`, then paste the token into the dashboard. Tokens are not placed
in URLs or persisted in browser storage.

`init` does not install R packages, edit PATH, open firewall ports, or overwrite an
existing configuration. `doctor` checks local configuration and package presence;
it does not certify compatibility or network readiness.

The generated configuration discovers applications on request. Deploy another app
by copying its folder into `demo/apps/`; nested app folders work too. No config
edit or server restart is needed for new folders.

## Two explicit runtime modes

**Native R:** ShinyHarbor starts the installed R interpreter, evaluates the real app
through `shiny::runApp`, and proxies HTTP/WebSocket traffic. The original application
is not transpiled. Worker limits, authentication and TLS are not commercially gated.
This mode does **not** reproduce all of Shiny Server's behavior.

**Upstream gateway:** ShinyHarbor forwards to an independently installed, unchanged
Shiny Server. That server continues to parse its original `shiny-server.conf` and
manage its application runtime. ShinyHarbor retains request paths and does not try
to translate or silently ignore directives. Standard proxy/header changes still
apply. This mode is not a proven zero-gap replacement or a native Windows port of
upstream Shiny Server, and it retains the upstream runtime's restrictions.

```toml
listen = "127.0.0.1:8080"
admin_listen = "127.0.0.1:8081"

[[upstreams]]
name = "existing-server"
mount = "/"
backends = ["http://127.0.0.1:3838"]
```

Separate routes can combine both modes. The longest matching mount wins.
For per-app analytics on an upstream, configure an upstream route for each app;
one `/` route groups all traffic under that route name.

## Source features

| Area | Implementation |
| --- | --- |
| Network | Tokio, axum/hyper, streaming bodies, pooled upstream HTTP connections, raw WebSocket tunnels |
| R processes | Lazy startup, per-app pools, session affinity, bounded concurrent startup, idle shutdown, draining `restart.txt` generations, process groups / Windows Job Objects |
| HTTPS | PEM certificates or rustls-acme TLS-ALPN-01 issuance, renewal and disk cache |
| Configuration | Strict TOML and a documented subset of `shiny-server.conf`, nested locations, request-time site discovery and static files |
| Access | Argon2id Basic passwords or trusted proxy SSO identities, user/group allowlists, native Shiny identity, per-worker secrets; separate private admin token |
| Bookmarks | Persistent server-side bookmark callbacks, scoped by application path and OS account |
| Analytics | SQLite aggregation, bounded nonblocking event queue, per-app traffic, connections, visitor-days, optional local country database |
| Operations | Readiness endpoint, Prometheus-format gauges, rotating worker logs, shutdown draining, service templates |
| Releases | Eight target jobs, native gateway tests, Linux/Windows/macOS real-Shiny browser jobs, checksums, resolved lockfile, dependency notices and SBOM |

These are implemented source paths, not claims of successful integration testing,
benchmarked performance, comprehensive security, or feature parity.

## Existing Shiny Server configuration

```sh
shinyharbor check --config examples/shiny-server.conf
shinyharbor serve --config examples/shiny-server.conf
```

Files ending in `.conf` use Shiny Server directive syntax. The current subset
supports multiple listeners/virtual hosts, nested `location` blocks, `app_dir`, `site_dir`, redirects,
directory listings, timeouts, the simple scheduler, error sanitization, frame
options, bookmark storage, log directories, transport selection and reconnect.
Unix `run_as` account switching includes supplementary groups; `user_dirs` and
opt-in `.shiny_app.conf` overrides are supported. Windows workers use the service
account. Unsupported directives fail with source positions; they are not silently ignored.
The full directive/runtime contract remains incomplete.

`site_dir` discovers apps when requested, including apps deployed after startup.
Static files support HEAD, byte ranges and modification-time cache validators.
Hidden files, source files and paths escaping the site root are rejected. For the
equivalent TOML configuration with multiple workers, see `examples/site.toml`.

## HTTPS and Let's Encrypt

```toml
listen = "0.0.0.0:443"
admin_listen = "127.0.0.1:3839"
apps_dir = "apps"
state_dir = ".harbor"

[tls]
mode = "acme"
domains = ["shiny.example.org"]
email = "admin@example.org"
production = false
```

Replace the example domain/contact. Point public DNS at the server and allow
inbound TCP 443. This implementation uses TLS-ALPN-01, not HTTP-01 or DNS-01.
A TLS-terminating CDN/proxy must not intercept the challenge. Wildcard and IP
identifiers are intentionally not supported by this configuration.

Start with staging (`production = false`), whose certificates browsers will not
trust. Set `production = true` for publicly trusted certificates after checking
reachability. Preserve `state_dir/acme`; it contains private account/certificate
material and supports reuse and renewal. The crate drives renewal automatically
while the server runs. ACME also starts an HTTP-to-HTTPS redirect listener on port
80. Set `redirect_http = false` to disable it behind another redirecting proxy, or
set `http_redirect_listen` to select a different address. Unknown hostnames are
rejected instead of reflected into redirect targets.

On Linux, use the limited bind capability in the systemd template rather than
running R as root. HTTPS terminates at this gateway; backend HTTP must stay on a
trusted local/private network. See [deployment](docs/DEPLOYMENT.md).

## Worker pools and access

```toml
listen = "127.0.0.1:3838"
admin_listen = "127.0.0.1:3839"
rscript = "Rscript"

[[apps]]
name = "sales"
mount = "/sales"
path = "apps/sales"
max_workers = 8
sessions_per_worker = 20
idle_seconds = 120
startup_seconds = 90
```

`max_workers` is a ceiling, not a promise to create that many processes. An
R process still has R's execution/concurrency characteristics. Work does not become
parallel merely because the HTTP gateway is asynchronous. Affinity is per browser
cookie and route; tabs sharing a cookie remain on the same worker. This is not
Shiny Server Professional's utilization scheduler or reconnect implementation.

For password protection, run `shinyharbor hash-password`, place the resulting PHC
hash in `[users]`, and add `allowed_users = ["alice"]` to the app. Never put plaintext
passwords into configuration or command arguments. See [configuration](docs/CONFIGURATION.md).

## Dashboard and privacy

![Dashboard preview using synthetic data, not a live server](docs/images/dashboard-preview.png)

The dashboard has application tables, an hourly chart, live connections, country
estimates, referrer origins, period selection, and responsive light/dark styling.
Its assets are embedded in the executable. No Node server, CDN, external analytics
service or frontend package installation is required.

“Page views” are successful HTML document responses. “WebSocket connections” include
reconnects, not distinct Shiny sessions. “Visitor-days” count daily keyed IP estimates
per app, not unique people. NAT, proxies and changing addresses affect counts.
Geography requires a separately obtained compatible country `.mmdb` file and is an
estimate, not a person's actual location. No country database is bundled.

Raw IP addresses, query strings, full referrer paths, cookies, passwords and request
bodies are not written to the analytics database. The database still contains
pseudonymous data. R apps and their logs can independently contain sensitive data.
`DNT: 1` and `Sec-GPC: 1` suppress analytics events. Analytics can be disabled entirely.
This is not a legal-compliance certification.

## Release targets

| OS | Architectures | Artifact targets |
| --- | --- | --- |
| Linux, glibc | x86-64, ARM64 | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` |
| Linux, musl | x86-64, ARM64 | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` |
| Windows, MSVC | x86-64, ARM64 | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` |
| macOS | Intel, Apple Silicon | `x86_64-apple-darwin`, `aarch64-apple-darwin` |

These are CI targets, not a promise of compatibility with every OS version or R
binary. R and its package libraries must be installed separately and match the
chosen R interpreter. Windows ARM64 may use an independently installed compatible
R runtime; building the Rust gateway for ARM64 does not establish native ARM64 R
package availability. No 32-bit, FreeBSD or other architecture support is claimed.

Push the repository to your GitHub account and use Actions. A tag matching the
Cargo version, such as `v0.1.0-alpha.1`, runs policy checks, all build jobs and browser
smoke checks before publishing a prerelease. Stable tags are blocked until reviewed
conformance evidence is present. See [releasing](docs/RELEASING.md).

## Repository map

- `src/`: configuration, security, authentication, proxy, worker pools, analytics, server and CLI.
- `r/`, `assets/`, `examples/`: launcher, dashboard, sample apps and configurations.
- `tests/`, `conformance/`: executable smoke checks and an explicit outstanding conformance inventory.
- `.github/`, `tools/`, `packaging/`: CI, releases, notices, checksums and service/install templates.

Start with [compatibility](docs/COMPATIBILITY.md), [security](SECURITY.md), and
[validation](VALIDATION.md). License: MIT, copyright Alek Rutkowski. Dependency and
external-runtime acknowledgments are in [ATTRIBUTIONS](ATTRIBUTIONS.md).
