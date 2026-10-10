# Compatibility and acceptance criteria

## Acceptance status

**The user's requirement of no compatibility gaps with Shiny Server is not met.**
This repository is an initial implementation, not a completed replacement. Nothing
in a successful build, dashboard screenshot, or five-feature browser smoke check
would justify calling it fully compatible. Stable release publication is blocked.

Reference information checked on 2026-10-09: CRAN Shiny 1.14.0, published 2026-06-21;
Shiny Server documentation/build 1.5.23.1030. The upstream development branches can
contain later changes. Exact source commits and runtime versions must be recorded
for any eventual conformance run. “Latest” also requires a continuously updated
acceptance run, not a one-time version label.

## What the native runtime does not yet establish

The native launcher uses the real installed Shiny package, but that alone is
insufficient for server compatibility. The outstanding surface includes:

| Surface | Current status |
| --- | --- |
| Complete `shiny-server.conf` syntax, scope, inheritance, overrides and diagnostics | Native parser and opt-in app overrides; unsupported directives fail. Complete grammar/directive equivalence remains open |
| SockJS fallback transports, protocol selection and session lifecycle | Seven real-browser modes pass; obsolete IE/XDomainRequest and exhaustive protocol edge cases remain unverified |
| Shiny Server reconnect contract and server-managed session IDs | Numbered replay, n/o session IDs, multiplexing and 15-second grace implemented; six wire cases pass against upstream and browser recovery preserves real R state |
| `run_as`, `user_dirs`, ownership, supplementary groups, login shell profiles | Unix UID/GID/groups, private data directories and Bash login profiles tested; user_dirs supported. Supervisors execute under the R account. PAM sessions and arbitrary Windows account switching remain open |
| Multiple original listeners/vhosts and template/log semantics | Multiple listeners/vhosts, custom directory/status-error templates, worker log modes/retention, worker-owned logging and HTTP access logs tested; specialized template/error contracts and merged log naming remain open |
| `restart.txt`, SIGHUP reload and reload-preserving sessions | Application reloads preserve real sessions and transfers; invalid changes roll back. Listener/TLS/global-setting changes require restart, so complete upstream reload parity remains open |
| Server-side bookmarking storage/access contract | Save/restoration across R workers and bidirectional import with Posit Shiny Server tested using one shared app path/account/store. Uploaded files, custom saved objects/sidecars and post-restore callbacks pass. Arbitrary existing stores and module-specific edge cases remain unverified |
| Complete app environment and session identity contract | Hosted runtime metadata, login profiles, supervisor arguments and Basic/proxy credentials tested. Complete upstream environment and native professional identity providers remain incomplete |
| R Markdown, widgets, caching, ranges, streaming and edge cases | Real reactive R Markdown and relative assets pass three browser routes; comprehensive rendering/caching equivalence remains unverified |
| Professional authentication providers, scheduler, access policy and monitoring | Proxy SSO, inherited user/group policies, load-factor worker scaling and idle-session limits tested; native PAM/LDAP/OIDC providers and licensed professional differential testing remain open |
| All operating systems and architectures | Five native build/package jobs passed on the initial commit; Windows x64 and Linux x64 GNU/musl tested locally. Corrected macOS builds and the complete current-source matrix remain unverified; see VALIDATION.md |
| Production TLS issuance and renewal | Actual local ACME challenge, trusted issuance, cache restart and automatic renewal passed; public CA deployment remains untested |
| Operational hardening, load testing, independent security review | Bounded queues and a 10,000-request optimized gateway load probe tested; broader load/failure testing and independent review remain open |

The list is not claimed exhaustive. `conformance/required.json` is a machine-readable
required-work inventory, not a completed specification or a list of passing tests.
There are intentionally no fake success records.

Additional accepted directives include `http_keepalive_timeout`, `access_log`, `set_header`,
`http_allow_compression`, `template_dir`, `log_file_mode`, `preserve_logs`,
`members_of`, `auth_proxy`, `required_user`, `required_group`,
`auth_ignore_case`, `whitelist_headers`,
`utilization_scheduler`, `app_session_timeout` and server `ssl`. They have candidate
runtime checks, with detailed scope in CONFIGURATION.md and VALIDATION.md.
Unix `socket_dir`, inherited `log_as_user`, `google_analytics_id` and
`frame_options allow-from` also have live candidate checks. Remaining configuration
gaps include complete listener/global-setting reload semantics. Independent TLS
certificates and mixed HTTP/HTTPS listeners now pass eight live checks; virtual
hosts sharing an address still require identical TLS settings. Unix socket mode
is unavailable on Windows, which uses private
loopback TCP workers. Exact log naming, directive edge cases and the complete
upstream default environment remain under comparison. Inherited `exec_supervisor`
supports quoted executable/argument prefixes; shell expansion requires a wrapper
script. Unix legacy configurations now use Bash login profiles, with precedence,
working-directory restoration and dropped worker privileges tested.

Top-level `set_header` supplies default public response headers, preserving values
already set by applications or the server. TOML uses `response_headers`. Changes
reload without replacing R workers. Private administration headers remain separate;
framing/hop-by-hop overrides and oversized header configurations are rejected.
This Professional-style directive has candidate tests, not a licensed Professional
differential result.

Large Shiny inputs, outputs and replay have real R/browser checks at 3 MiB over
WebSocket and XHR polling. The default encoded message limit is 64 MiB, with
configurable session replay and shared queue budgets. This removes the earlier
1 MiB gateway and 2 MiB browser-buffer restrictions; it does not establish
unlimited message sizes or exhaustive compatibility at every boundary.

## Native configuration subset added 2026-10-07

Original positional CLI startup (`shinyharbor FILE.conf`), local default config
search and explicit `--config` startup are supported. On Unix the search also
includes `/etc/shiny-server/shiny-server.conf`. The legacy `R` environment variable
selects the default interpreter; `r_path` overrides it.

### Bookmark migration checks

`tests/bookmark_migration_host.py` creates isolated copies of `examples/session`
and starts an independently installed Posit server alongside ShinyHarbor, under
the same Unix account with one app path and bookmark root. It runs a real browser
test that saves numeric input on each host and restores it through the other:

```sh
python tests/bookmark_migration_host.py --binary target/debug/shinyharbor \
  --reference-binary /opt/shiny-server/bin/shiny-server
```

Install the browser test dependencies first. The reference server is not bundled.
For a browser running outside the Unix host, `--serve-only` writes the two URLs to
`test-results/bookmark-fixture.json`; pass them to `tests/bookmark_migration.py`
with `--reference` and `--candidate`. Create `test-results/bookmark-fixture.stop`
to stop the pair. Fixtures also expire after five minutes and remove their test
store. Production bookmark data is never used by this harness. These cases passed
against Posit Shiny Server 1.5.23.1030 with Shiny 1.14.0 on both hosts.

### Parser and runtime scope

The `.conf` loader supports `server`, `listen`, nested `location`, `app_dir`,
`site_dir`, `directory_index`, `redirect`, `app_init_timeout`, `app_idle_timeout`,
`simple_scheduler`, `sanitize_errors`, `frame_options` (allow/deny/sameorigin),
`bookmark_state_dir`, `log_dir`, and `run_as :PROCESS_USER:`. Deprecated leaf
`application` directives are ignored as documented by upstream. This is a subset,
not a complete configuration translator. TOML remains available for worker pools,
TLS, analytics, native authentication and upstream routes.

Sites are resolved on request, with lazy pools for nested apps and R Markdown
documents. A site can contain static HTML and assets; range, HEAD and conditional
GET are delegated to tower-http. R sources, hidden paths, Windows device names and
paths escaping the configured root are rejected. App mounts and directory indexes
support spaces and Unicode names using canonical percent-encoded URL segments;
four real app routes pass reactive/upload/download checks with these names.
At most 4096 discovered app pools are retained per instance. Discovery includes
`user_dirs` and application configuration reloads; complete R Markdown path
semantics and exhaustive encoded-URL equivalence remain under review.

Existing WebSocket sessions keep their process when `restart.txt` changes. New
document requests select the new generation; old generations stop after becoming
idle. Per-page worker tickets and R session-token routing keep shared-cookie tabs
attached to their own workers; real upload/download tests cover this behavior.
`tests/native_compat.py` exercises this alongside persistent bookmark restoration
through the Rust server; it has passed locally and is wired into CI.

The directly executed `tests/launcher.py` covers real Shiny HTTP, WebSocket
reactivity, trusted identity, rejected missing/forged worker secrets, bookmark
saving and restoration after an actual R restart. It bypasses Rust and is not
evidence of Rust gateway conformance.

## Upstream gateway mode

For an existing deployment, preserving the actual upstream runtime is safer than
implementing a permissive partial configuration translator. An upstream route
leaves original path prefixes intact and sends traffic to unchanged Shiny Server.
The original server remains responsible for its directives, users, processes and
SockJS handling. It must be installed and configured independently; it is not
bundled or silently downloaded.

This is **not** a substitute for proving proxy transparency. ShinyHarbor normalizes
hop-by-hop headers, sets trusted forwarding information, strips several untrusted
identity headers, sets its own affinity cookie and introduces explicit deadlines.
Those choices must be evaluated against the actual deployment. A proxy-auth setup
that relies on stripped identity headers needs a separately specified trust design.
The upstream remains subject to its original operating-system constraints and
edition limits. A Rust gateway on Windows does not port upstream Unix behavior to
Windows. Multiple upstream replicas require identical paths and state planning;
active R sessions are not migrated or made highly available.

Per-app analytics require explicit per-app routes. A root upstream route sees one
logical route, not automatically every application discovered behind it.

## Required conformance work

Use independently deployed, pinned upstream and candidate systems with the same
application fixtures, R libraries, environment and browser versions. Compare
observable protocol/configuration behavior, including unsuccessful and adversarial
cases. The matrix must cover every documented directive, all supported transports,
reconnect and bookmarking, user/permission semantics, reload/termination, upstream
headers/cookies, malformed input, and each advertised platform. Test professional
parity only against a legally available reference and a written feature scope.

`tools/differential_smoke.py --reference URL --candidate URL` executes the same five
browser features against both servers. Passing it provides narrow evidence only;
it does not satisfy the inventory. The full suite is not implemented. Extending it
requires actual test fixtures and results, not changing JSON booleans.

The stable gate requires implemented mandatory cases, a committed lockfile,
source-bound result metadata, retained evidence hashes and an identified reviewer.
These controls are necessary workflow checks, not a mathematical proof that all
possible app behavior is equivalent. Evidence files must be explicitly added to
git for a stable tag; ignored local files do not reach the release runner.

## Primary references

- https://docs.posit.co/shiny-server/
- https://github.com/rstudio/shiny-server
- https://raw.githubusercontent.com/rstudio/shiny-server/master/config/shiny-server-rules.config
- https://cran.r-project.org/web/packages/shiny/index.html
- https://github.com/rstudio/shiny

## Alpha.3 scope

Inherited case-insensitive user/group authorization and additional Shiny session
headers are now implemented and tested with live R. Header forwarding applies
to SockJS and reconnect/multiplexed channels; reserved identity/framing headers
remain managed by the server. No licensed Professional instance was available.
Native PAM/LDAP/AD/OIDC/password-file login providers, full configuration/reload
equivalence and exhaustive transport behavior remain incomplete. This release
does not claim full Shiny Server or Professional drop-in compatibility.

Alpine packaging adds OpenRC rather than assuming systemd. Its native package
requires apk-tools 3. macOS now has a prepared system LaunchDaemon, not merely
a per-user login agent; native macOS/ARM acceptance remains pending.
