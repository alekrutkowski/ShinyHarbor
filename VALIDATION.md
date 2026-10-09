# Delivery-time validation

## 2026-10-08 executable validation

Rust 1.99.0 builds succeeded on Windows x64 and WSL Ubuntu x64. The Windows
optimized build also succeeded. Installed R was 4.6.1 and Shiny was 1.14.0.
An independently installed Posit Shiny Server 1.5.23.1030 provided the reference.

| Executed check | Result | Scope |
| --- | --- | --- |
| Rust unit tests | 29 Windows / 31 Linux passed | Includes configuration, replay state, worker capacity, idle tunnel timers, routing, SQLite, identity/header protection; Linux additionally tests symlink escape and OS deployment groups |
| Clippy | Passed with `-D warnings` | Windows and Linux all-targets |
| Python repository/SQLite tests | 18 passed | Actual SQL and repository/workflow checks |
| Live gateway | 14 checks passed on Windows and Linux | Streaming, text/binary WebSockets, forwarding trust, cookies, real analytics and dashboard headers |
| Real Shiny transports | Seven browser modes passed | WebSocket, XHR streaming/polling, eventsource, htmlfile, iframe polling and JSONP; each tests reactivity, Unicode, uploads, downloads and dynamic UI |
| Upstream differential smoke | Equal results | The same five browser features on reference and candidate |
| Reconnect | Nine checks passed | Browser offline recovery preserves the R session and UI state; replay, multiplexing, duplicate open, unknown and expired sessions; six wire cases also passed on the reference |
| Shared-cookie multiworker tabs | Four checks passed | Distinct R workers and correct per-session upload/download routing |
| Native compatibility | 13 checks passed | Legacy config, static HTTP, dynamic discovery, bookmarks, frame options and restart generation draining |
| Virtual servers | Six checks passed | Multiple listeners and hostname routing |
| Authentication proxy | Ten checks passed | Real R users/groups, legacy directives, anonymous access, untrusted/duplicate header denial, app overrides, multiplex ACLs and changed-group reconnect denial |
| Interactive R Markdown | Three browser cases passed | Discovered index, explicit document path and configured app, with reactive updates and relative resources; rmarkdown 2.32 and Pandoc 3.12.1 |
| Resource limits | Seven checks passed | Load-factor scaling, worker ceiling, overload and recovered capacity, plain/robust idle expiry, activity resets and heartbeat exclusion |
| Unix account switching | Five checks passed | Named fallback, worker UID, supplementary groups, private gateway keys and user-owned bookmark directories |
| Live dashboard | Six checks passed | Real R traffic, real SQLite API, page/live counts, token unlock/lock, no persistent browser token |
| Configuration reload | Seven Windows browser checks and two Linux signal checks passed | Added/removed routes, worker reuse, generation draining, surviving sessions/transfers, CLI/admin authorization and invalid-change rollback |
| HTTP compression | Six checks passed | Gzip/deflate negotiation, decoded content, disabled mode, small responses and ranges; all seven browser transports also pass with compression enabled |
| Unicode app paths | Six checks passed | Directory labels/links, four explicit/discovered Unicode/space/ampersand app routes with reactivity/uploads/downloads, equivalent escaping and traversal denial |
| Worker logs | Three Windows / four Linux checks passed | Successful/nonzero exit retention, preservation markers, stderr forwarding and Linux file permissions |
| Custom templates | Eight checks passed | Directory/error rendering, escaping, inheritance, specific error fallback, live file changes, invalid syntax, protected configuration/log/state files |
| HTTP access logs | Seven checks passed | Legacy/default and named formats, token validation, streaming completion, log rotation/reload, escaped fields, no password logging and loss metrics |
| HTTP inactivity | Five checks passed | Persistent traffic resets the deadline, idle/partial requests expire, zero disables the timeout |
| Unix worker sockets | Six checks passed | Real R HTTP, native WebSocket and SockJS reactivity, private socket permissions, shared-secret denial, shutdown cleanup; the separate root-to-user suite also passes with sockets |
| Worker-owned logging | Six checks passed | Root service delegates creation, permissions, bounded rotation and exit-code retention to the R account; NFS itself not tested |
| Page settings | Four checks passed | Inherited allow-from, Google tag markup, disabling inherited settings and script-injection rejection; no external analytics requests |
| Local ACME/TLS | Twelve checks passed | Actual ALPN challenge, trusted issuance, TLS 1.2/1.3, redirect security, cache restart, automatic renewal, valid/invalid PEM replacement and legacy ssl directive |
| Per-listener TLS | Eight Windows checks passed | Independent trusted certificates, mixed HTTP/HTTPS, isolated certificate reload, hidden private keys, TOML inheritance, forwarded scheme, secure cookies and conflicting shared-address settings |
| Optimized gateway load probe | 10,000 requests, 32 concurrent, zero errors | 2,790 requests/s, p50 11.33 ms, p95 11.84 ms, peak sampled RSS 16.3 MiB; 1 KiB loopback responses, Python client/backend, compression negotiation and analytics enabled |

The load probe is an environment-specific measurement, not isolated gateway
throughput or a measure of R application capacity. TLS testing used Pebble 2.10.1
with shortened certificate lifetimes and real challenge validation; no public CA
was contacted, no certificate verification was disabled, and system trust was not
modified. Other CPU/OS release targets and a licensed Professional reference have
not been executed here. Full compatibility remains unverified.

Reproduce the new suites with `tests/transports.py`, `tests/reconnect.py`,
`tests/multiworker.py`, `tests/live_dashboard.py`, `tests/configuration.py`,
`tests/tls.py`, and `tests/load.py`, each using `--binary` with a built executable.
`tools/fetch_pebble.py` installs checksum-pinned test tooling into `.build` for the
TLS suite. Raw reports and logs are in `test-results`; a retained machine-readable
summary is in `validation/2026-10-08.json`.

Local debug/incremental artifacts were cleaned when C: was low on space. Ignored
`.cargo/config.toml` settings now disable development debug symbols and incremental
compilation; these machine-local settings do not change release settings or CI.

## 2026-10-07 continuation

The repository now includes native `.conf` parsing for a documented subset,
request-time site discovery/static serving, worker generation draining for
`restart.txt`, persistent bookmarks, native session identity and per-worker
WebSocket secrets. Git was initialized locally on `main`; no remote was created
and no release was published.

| Check executed here | Result | Scope |
| --- | --- | --- |
| `Rscript --vanilla tests/runtime.R` | 5 tests, 35 assertions passed | Actual Shiny callback registration, identity, secret checks, bookmark isolation/failure cleanup and Windows path normalization |
| `python tests/launcher.py` | 7 live checks passed | Actual R HTTP/WS server, reactivity, `session$user`, rejected missing/forged secrets, bookmark save and restoration after terminating/restarting R |
| Python repository and SQLite unit tests | 17 passed, 1 skipped | Workflow test needs unavailable PyYAML; all other tests passed, including actual extracted SQL |
| Workflow YAML parsing via installed R `yaml` | All 3 workflows parsed | Syntax and jobs structure; not an Actions execution |
| Dashboard JavaScript syntax | Passed | RStudio-bundled Node `--check assets/app.js` |
| Python syntax | Passed | `python -m compileall -q tools tests` |
| POSIX installer syntax | Passed | Git-bundled Bash `-n packaging/install.sh` |
| Rust build, Rust tests, rustfmt, Clippy | Blocked | Rustup shims exist but no accessible installed toolchain. Workspace-local installation failed when network access to static.rust-lang.org was denied (Windows error 10013) |
| Rust gateway and native browser suites | Not executed locally | Python browser dependencies and a compiled Rust executable are unavailable |
| Linux/Windows/macOS browser CI, eight release targets | Configured, not executed | Includes `tests/native_compat.py` for legacy configuration, static HTTP, dynamic discovery, restarts and bookmark restoration through Rust |

The launcher test starts R directly. It proves the R integration cases above,
not that the Rust binary builds, routes traffic correctly, or matches Shiny Server.
No compatibility case has been promoted to fully implemented/certified on the
basis of these checks. Full SockJS/reconnect, configuration and platform acceptance
remain unresolved. The machine-readable record is `validation/2026-10-07.json`.

## Previous preparation record

Date: 2026-10-02. This file separates executed checks from work supplied for CI.
The software is alpha source, not a compiled or compatibility-certified release.

| Check | Result | Scope |
| --- | --- | --- |
| Repository/configuration/workflow structural tests | 13 passed | TOML/YAML parsing, target inventory, required files, embedded assets, gate behavior and checksum policy |
| SQL extracted from `src/analytics.rs` | 5 passed | Actual SQLite schema, upserts, visitor deduplication, queries and retention logic, via Python SQLite |
| Dashboard browser checks | 11 passed | Actual HTML/CSS/JavaScript with synthetic API responses, rendered in Chromium without network navigation |
| JavaScript syntax | Passed | `node --check assets/app.js` |
| Python syntax | Passed | `compileall` for supplied tools and tests |
| Shell installer syntax | Passed | `sh -n packaging/install.sh` |
| Source archive/digest and local Git bundle | Verified during packaging | File presence and archive integrity, not executable correctness |
| Rust compilation, rustfmt, Clippy and Rust unit tests | Not executed | No functioning Rust toolchain was available in the preparation environment |
| Executable gateway and real-Shiny integration suites | Supplied, not executed | Must run against compiled binaries in CI |
| All release-platform jobs and Windows SCM lifecycle | Not executed | Target matrix is configuration, not successful platform evidence |
| Real ACME issuance/renewal, public HTTPS, independent security review | Not executed | Integration code exists, but these behaviors are unverified |
| Complete Shiny Server conformance | Not implemented / not verified | Zero-gap acceptance criterion is not met; stable gate blocks publication |

The first HTTP-served dashboard browser attempt was blocked by the preparation
browser's managed URL policy (`ERR_BLOCKED_BY_ADMINISTRATOR`). The subsequent local
UI check rendered the same asset source in memory and mocked only the API. No
browser policy was modified. This check **did not** exercise server HTTP responses,
CSP enforcement, real token validation, real database/API integration, or R behavior.
The normal CI dashboard job uses the HTTP-served mode with CSP.

The screenshots are explicitly labelled **synthetic preview, not live data**. They
demonstrate layout and interactions, not a live deployment or performance benchmark.
The machine-readable local report is in `validation/local-report.json`.

Reproduce local checks from the repository root:

```sh
python -m unittest discover -s tests -p 'test_*.py' -v
node --check assets/app.js
python -m compileall -q tools tests
sh -n packaging/install.sh
# Standard browser check, where local HTTP navigation is permitted:
python tests/dashboard.py
# Asset-only mode for an environment that prohibits browser navigation:
HARBOR_UI_IN_MEMORY=1 CHROMIUM_EXECUTABLE=/path/to/chromium python tests/dashboard.py
```

Compile and run actual server checks separately:

```sh
cargo fmt --all
cargo test --all-targets
cargo build
python tests/gateway.py --binary target/debug/shinyharbor
python tests/shiny_browser.py --binary target/debug/shinyharbor
```

Use the `.exe` path on Windows. Current CI applies the platform target and shared
lockfile explicitly. A successful smoke run does not turn the outstanding full
conformance cases into passing tests.
