# Operations and metric semantics

## State and backup

`state_dir` contains `secret.key`, `admin.token`, `instance.lock`, `bootstrap.R`,
`run/`, `logs/`, `acme/`, and `analytics.sqlite` with SQLite WAL/SHM files. Only one
server instance may use a state directory. Do not mount the same state directory
into multiple active replicas. Backup procedures must preserve private permissions.

For a simple consistent backup, stop the service, copy the state directory, then
restart. Do not copy only the live `.sqlite` file and discard a nonempty WAL. Use
SQLite's online backup API/tooling for live backups after reviewing its behavior.
Restore the signing key with its state if existing upstream affinity must survive.
Native R processes themselves are not restored by restoring files.

ACME state must survive restarts and replacements. Test initial issuance against
staging. ACME failures are logged, not silently converted to plaintext. Existing
valid cached certificates can continue to serve while renewal is retried by the
library. Successful production issuance and renewal have not been demonstrated by
the preparation-time validation report.

## Shutdown and restarts

Console Ctrl+C and Unix SIGTERM request draining. The app listener stops accepting
new work, tracked upgraded connections are given the configured grace interval,
then remaining tunnels are closed and workers stopped. Native process trees use
Unix process groups or Windows Job Objects. Cleanup has additional bounded waits;
the config grace value is not an exact total shutdown deadline.

A process restart terminates native R sessions; state does not migrate between
processes. Native transport reconnects preserve sessions across short network
interruptions. `restart.txt` drains old workers, and `shinyharbor reload` (or Unix
SIGHUP) reloads application routing/settings while existing sessions continue.
Invalid reloads keep the working configuration. Listener, TLS and global settings
still require a restart; see CONFIGURATION.md for the supported reload scope.
Root upstream mode leaves backend process management with the original server.

## Logs and health

Each native worker writes separate stdout/stderr logs. Files rotate at 10 MiB with
one backup. Successful process exits remove their logs unless `preserve_logs`
is enabled; failed exits retain them. Preserved logs carry `.log.keep` markers
and are excluded from periodic age pruning. Other old logs in the default state
log directory are pruned according to retention days; custom log directories need
an operator-managed retention policy. This is not a global disk-space ceiling. Native
apps can emit secrets into their logs. The Windows service's own append-only log
needs an external retention policy. Unix server output goes to stderr/journald.

`log_file_mode` applies explicit Unix permissions, ignoring umask (default `0640`).
Windows files inherit their directory ACLs. A nonempty `SHINY_LOG_STDERR` environment
variable also relays worker stdout/stderr to the server's stderr. Configured worker
log directories and log artifacts cannot be served as static site files. The legacy
`preserve_logs` and `log_file_mode` directives and their opt-in app overrides are
supported; upstream log naming/merged-output and `log_as_user` are still outstanding.

`GET /healthz` on the admin listener means “admin listener is responding”, not
“every app is already initialized”. Lazy apps start on demand. Protected
`GET /api/status` reports pool counts and an explicit `not-certified` compatibility
status. Protected `GET /metrics` exposes live WebSocket/worker gauges and dropped
analytics events. No Prometheus dependency is required in the server.

For upstream routes, the displayed worker count is the configured backend count,
not a successful backend health probe or a count of their underlying R workers.

## Reading usage statistics correctly

| Metric | Definition and limitations |
| --- | --- |
| Requests | Requests reaching the instrumented proxy response/error paths; not every rejected/admin/static-index request |
| Page views | Successful HTML responses to requests recognized as browser documents/frames or HTML-accepting GETs; not unique visits |
| Visitor-days | Daily HMAC-derived IP estimates per route, counted once per day after a page view; summed across days, not people |
| App connections | Native R connections across all SockJS transports, plus raw upstream WebSocket upgrades; robust transport reconnects preserve the existing native connection |
| Live connections | Active R connections and raw upstream WebSocket tunnels; includes native sessions held during reconnect grace |
| Worker count | Live native workers, or configured upstream endpoints |
| 5xx errors | Upstream 5xx and proxy acquisition/connection/header-timeout errors recorded by the proxy |
| Traffic | Application HTTP body and socket payload bytes before optional HTTP compression, excluding framing; interrupted raw socket traffic can be undercounted |
| Header time | Time until response headers, including acquisition/startup and upload; not time to complete a download |
| Countries | Optional local IP-country lookup; VPN/NAT/proxy results are estimates; `ZZ` is unknown |
| Referrers | HTTP(S) origins only; missing values become Direct / unknown |

Hourly/daily bucket boundaries are UTC and reporting intervals include partial
boundary buckets. The chart is hourly UTC; the “updated” time uses the browser's
local display. SQLite retention bounds time, not total traffic cardinality or disk
usage. An overloaded event queue intentionally drops data rather than delaying app
traffic. Dropped events reset on process restart and are shown in the UI.

Collection honors DNT/GPC and can be disabled; historical records are not deleted
by toggling `enabled`. Snapshot queries flush queued events before reading. There
is no hosted analytics service, remote geolocation API, user-agent fingerprinting,
user identity analytics, or exact distinct-user tracking.

## Performance evaluation

A 10,000-request loopback gateway load probe has run; see VALIDATION.md for
the environment, measured latency/throughput and sampled memory. This does not
measure R application capacity or establish a production sizing guarantee.
Benchmark real workloads with fixed
R/package versions, session duration, websocket/HTTP mix, worker settings, CPU/RAM,
TLS mode, p50/p95/p99 latency, dropped events and failure behavior. Measure gateway
resources separately from R workers and include idle connections and slow clients.
