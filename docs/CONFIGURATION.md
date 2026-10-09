# Configuration

All paths are relative to the configuration file unless absolute. Bare `Rscript`
is searched through the service account's PATH; a path containing components is
resolved from the config directory. Unknown TOML fields are errors. Application
routes, access policies and response headers support live reload; listeners and other global settings
currently require a restart. See the reload section below.

Files with a `.conf` extension select the native Shiny Server syntax parser.
See `examples/shiny-server.conf` and the supported subset in
[compatibility](COMPATIBILITY.md). Other extensions select TOML.

Both `shinyharbor serve --config shiny-server.conf` and the original-style
`shinyharbor shiny-server.conf` start the server. `shinyharbor --config FILE` is
also accepted. Without an explicit path, commands look for `harbor.toml`, then
`shiny-server.conf` in the working directory, then the installer data directory,
files alongside the executable, per-user settings and system locations.
`SHINYHARBOR_CONFIG` overrides this search. Unix system locations include
`/etc/shinyharbor/harbor.toml` and `/etc/shiny-server/shiny-server.conf`.
Windows service installation still uses an
explicit path. Legacy configurations use the `R` environment variable to select
the executable, falling back to `R` on PATH; an inherited `r_path` directive
takes precedence. TOML continues to use its `rscript` setting.

| Top-level key | Default | Meaning |
| --- | --- | --- |
| `listen` | `127.0.0.1:3838` | App listener address and nonzero port |
| `admin_listen` | `127.0.0.1:3839` | Must be loopback; separate admin HTTP listener |
| `state_dir` | `.harbor` | Exclusive instance directory; contains secrets and analytics |
| `rscript` | `Rscript` | Default R interpreter |
| `r_library` | absent | Additional R library, resolved relative to this file; setup can install Shiny here |
| `apps_dir` | absent | Scan immediate app subdirectories at startup |
| `trusted_proxies` | `[]` | CIDRs allowed to supply forwarding information |
| `public_origin` | absent | HTTPS origin when TLS terminates at an external trusted proxy |
| `allow_plaintext_public` | `false` | Explicit opt-in needed for non-loopback plaintext |
| `shutdown_grace_seconds` | `30` | Maximum WebSocket drain interval, with additional HTTP/process cleanup |
| `response_header_timeout_seconds` | `120` | Positive deadline for an upstream to return response headers; includes upload time |
| `max_inflight` | `8192` | Active proxied requests/tunnels, not all accepted TCP sockets |
| `response_headers` | `{}` | Default headers for public HTTP responses; application/server values take precedence |

Keep a TLS-terminated backend private even when setting `public_origin`. This is an
operator assertion about deployment, not cryptographic proof that every incoming
connection used HTTPS. TLS off is appropriate for local development; public
plaintext requires an explicit choice and still does not safely transport credentials.

The default `Rscript` is discovered on PATH, through `R_HOME`, or in standard R
installation directories on Windows/macOS/Linux. Explicit executable paths take
precedence. `r_library` is passed through `R_LIBS`, preserving the service account's
usual libraries; explicit app `environment.R_LIBS` overrides this value. The
configured R library is protected from static file serving.

## Public response headers

```toml
[response_headers]
X-Deployment = "analytics"
Referrer-Policy = "same-origin"
```

The original syntax is top-level `set_header X-Deployment "analytics";`.
Repeat the directive for additional headers. Legacy header names are case
insensitive and the last value for a name wins. TOML rejects duplicate names,
including differently capitalized names.

Headers apply to public static and app responses, errors, WebSocket handshakes
and the HTTP-to-HTTPS redirect listener. Existing headers from the application or
server take precedence. The separate private administration listener keeps its own
policy. A live reload updates these defaults without replacing R workers or closing
existing sessions. Already-started responses retain their original policy.

Names and values must be valid HTTP headers. The configuration permits up to 64
headers and 16 KiB of combined name/value bytes. Framing and hop-by-hop headers
such as `Content-Length`, `Transfer-Encoding`, `Connection` and `Upgrade` are
managed by the server and rejected here. These limits are intentional differences
from an unrestricted `set_header` implementation.

## Native app

```toml
listen = "127.0.0.1:3838"
admin_listen = "127.0.0.1:3839"
# Windows example; replace with the actual installed version/path:
# rscript = 'C:\Program Files\R\R-x.y.z\bin\Rscript.exe'

[[apps]]
name = "sales"
label = "Sales dashboard"
mount = "/reports/sales"
path = "apps/sales"
max_workers = 4
sessions_per_worker = 20
idle_seconds = 60
startup_seconds = 60
# rscript = "/opt/R/bin/Rscript"
# allowed_users = ["alice"]

[apps.environment]
MY_APP_SETTING = "value"
```

Names allow ASCII letters, numbers, `_`, `-` and `.`, excluding `.` and `..`.
Mounts must begin with `/`, use safe segments and have no trailing slash except
root. Two routes cannot use the same mount or name. Longest matching mount wins.
Visiting a non-root mount without its final slash causes a 308 redirect.

`apps_dir` discovers immediate directories containing `app.R`, `app.r`, `server.R`
or `server.r`. It does not reproduce recursive `site_dir`/`user_dirs` behavior.
An explicit app of the same name takes precedence. Explicit paths can point to
an app directory or an `.Rmd` file; R Markdown additionally needs `rmarkdown` and
its own runtime dependencies. `HARBOR_*` environment variables are reserved.
R startup files are not disabled, allowing project profiles such as renv; inspect
those files as executable code before deployment.

`idle_seconds = 0` disables idle worker shutdown. Sessions are approximated by
active WebSocket connections for the native worker ceiling. Affinity may keep a
browser on a full worker even when another exists. Session capacity is not a strict
R CPU/memory quota and not a SockJS session counter.

Additional app options are `sanitize_errors = true`,
`frame_options = "allow"` (also `deny` or `sameorigin`), `log_dir`, and
`bookmark_state_dir`. The last two default to `state_dir/logs` and
`state_dir/bookmarks`. Custom log directories use Harbor's per-worker filenames
and size rotation; arrange retention for custom directories yourself.
Keep the bookmark directory across deployments. Bookmarks are scoped by app path
and the R process's OS account, not by HTTP Basic username. Changing either can
change which saved bookmarks are visible.

Create or update `restart.txt` inside an app directory to retire its current
worker generation. New page loads use new workers while existing WebSocket
sessions finish in the old workers. Retiring workers do not count against the
new generation's worker ceiling and exit after their active requests finish.

## Sites and nested apps

```toml
[[sites]]
name = "reports"
mount = "/reports"
path = "apps"
directory_index = true

[sites.app_defaults]
max_workers = 4
sessions_per_worker = 20
startup_seconds = 90
frame_options = "sameorigin"
# allowed_users = ["alice"]
```

Sites discover nested `app.R`/`server.R` directories, `index.Rmd` documents and
explicit `.Rmd` paths at request time. Apps added after startup become available
without restarting the server. Explicit nested routes take precedence. Site
access restrictions also protect its static files. Static responses support
GET, HEAD, ranges and modification-time cache validators; directory listings
are optional and `index.html` takes priority. Dotfiles, R source/data files and
paths outside the site root are denied. Site contents and apps must be writable
only by trusted deployers; this is not a sandbox for hostile application code.

## Upstream

```toml
[[upstreams]]
name = "existing-sales"
mount = "/sales"
backends = ["http://127.0.0.1:3838"]
```

Use HTTP origins without paths, credentials, fragments or queries. Request paths
are **not** stripped. An upstream should therefore host `/sales/`, not only `/`.
Backend TLS is not implemented; use trusted local/private connectivity. Multiple
backends use round-robin assignment plus signed affinity. Failed requests are not
replayed on another server. This deliberately avoids duplicating side effects but
does not provide automatic failover or active-session migration.

## TLS

Top-level `http_allow_compression = true` enables gzip/deflate negotiation for
ordinary HTTP responses. The legacy root directive has the same name. Small
responses, images, server-sent events, range responses and `Cache-Control:
no-transform` responses are excluded. SockJS streaming responses remain uncompressed
so small messages are delivered promptly. Analytics traffic measures application
body bytes before this optional HTTP encoding, not exact network bytes.

```toml
[tls]
mode = "pem"
cert = "certs/fullchain.pem"
key = "certs/privkey.pem"
```

PEM files are read at startup and checked for changes every five seconds. Valid
replacements take effect without restarting or disconnecting existing sessions;
invalid replacements retain the last usable certificate. The ACME mode described
in README drives its own automatic renewal and persists its account and certificate
under `state_dir/acme`.

Legacy configuration accepts `ssl key_path cert_path;` inside a `server` block.
Different listening addresses can use different certificates or mix HTTP and HTTPS
in the same instance. TOML `[[servers]]` entries inherit root `[tls]` by default;
set `tls = { mode = "off" }` or a nested `[servers.tls]` PEM configuration to override
that listener. Certificate files reload independently. Virtual servers sharing
the same IP and port must use identical TLS settings; separate SNI certificates
on a shared listener are not implemented.

ACME enables a port-80 redirect listener by default. Top-level `redirect_http = false`
disables it; `http_redirect_listen = "127.0.0.1:8080"` overrides its binding. For PEM
mode, set `http_redirect_listen` and `public_origin` explicitly to enable redirects.
The listener preserves paths, query strings and methods using HTTP 308.

For private ACME authorities, `[tls]` accepts `directory_url` (HTTPS) and an optional
`ca_bundle` PEM file trusted only for that authority's API requests. Certificate
validation is never disabled. Custom authorities may use a different local HTTPS
port; public Let's Encrypt TLS-ALPN validation still requires reachable port 443.

`mode = "off"` is the default. ACME currently supports one listener per instance;
other listeners can use PEM certificates or HTTP. Virtual hosts may share the
ACME listener and its multi-domain certificate.

## Transport recovery

`load_factor` controls when a new request triggers another R worker, up to
`max_workers`; its range is greater than zero through one (default one).
`sessions_per_worker` bounds concurrent native sessions. The original
`utilization_scheduler maxRequestsPerProc loadFactor maxProc` directive maps to
these settings. Worker selection favors the fewest open sessions and preserves
explicit worker affinity. A restarted generation has its own worker allowance.
This is request-triggered scaling, without CPU-based scheduling or session migration.

`session_timeout_seconds` (legacy `app_session_timeout`) closes an idle native
session after the configured seconds; zero disables the limit. Application data
in either direction resets the timer. SockJS heartbeat and replay acknowledgments
do not keep an idle R session alive. The setting works for ordinary and reconnecting
SockJS connections and direct native WebSocket tunnels. Per-app overrides may set
the scheduler, session timeout, and reconnect policy.

Native apps use the embedded SockJS client, including HTTP streaming and polling
fallbacks. `disable_protocols` accepts upstream protocol names and the `streaming`
and `polling` groups. `reconnect = true` is the default. Logical sessions survive
transport loss for 15 seconds, with numbered acknowledgments and bounded replay
buffers; the browser displays recovery status. Recovery preserves the R session,
and does not migrate it to another R process or survive a server restart.

The upstream `n=`/`o=` session URL parameters and multiplex channel framing are
supported. Authentication is checked for each channel's application. Per-page
worker tickets and R session identifiers keep uploads/downloads attached to their
own workers when browser tabs share cookies. `reconnect = false` uses ordinary
SockJS without retaining a disconnected WebSocket session for recovery.

## Reloading configuration

After editing your configuration, run `shinyharbor reload --config harbor.toml`.
On Unix, SIGHUP also reloads the original configuration file. The private admin
API offers `POST /api/reload`, authenticated with the existing admin Bearer token.
The server reads and validates the complete file before publishing the new routing
and access policy snapshot. Invalid changes leave the working configuration intact.

App, site, redirect, upstream, R executable, authentication and response-header changes can be
reloaded. Unchanged apps keep their workers. Changed app workers drain existing
sessions; new sessions use the new settings. Signed worker tickets and session
paths keep existing uploads/downloads routable. Removed routes reject new requests,
including new multiplex channels over an already-open transport. Existing sessions
are not forcibly logged out by an access-policy change.

Listener addresses, virtual-server definitions, TLS mode/domains, state directories,
analytics configuration and global transport limits currently require a restart.
A reload containing those changes is rejected as a whole; it is not partially
applied. PEM certificate file contents reload automatically, independently of this
command. This covers application reloads, not complete upstream SIGHUP equivalence.

For Unix `user_dirs`, `members_of` restricts deployment to OS users belonging to
one of the named groups (including their primary group). It is independent of
viewer `allowed_groups`. The legacy directive is inherited and an empty directive
lifts the inherited restriction. TOML uses `sites.app_defaults.members_of`.
Windows rejects this option because Unix group lookup is unavailable.

## Analytics

```toml
[analytics]
enabled = true
retention_days = 30
queue_capacity = 8192
# country_database = "GeoLite2-Country.mmdb"
```

Retention is 1..366 days. Hour/day buckets can retain a partial boundary bucket
beyond the exact second-based cutoff. Queue events are dropped rather than blocking
proxy traffic when overloaded; the dashboard exposes the count. Country records
must expose `country.iso_code`. Missing lookups become `ZZ`. Database updates are
operator-managed and loaded on restart. No third-party network lookup occurs.

## Password protection

Generate a hash using `shinyharbor hash-password` with interactive hidden input.
Add `allowed_users = ["alice"]` to the selected app before starting another table:

```toml
[users]
alice = '$argon2id$v=19$m=19456,t=2,p=1$REPLACE_WITH_REAL_GENERATED_HASH'
```

The example placeholder is not a valid credential. Unknown users and malformed
hashes fail config validation. HTTP Basic invokes the browser's password prompt.
Only successful password checks are briefly cached using a keyed digest; hashing
work is concurrency-limited. Native authenticated sessions receive `session$user`
and an empty `session$groups` list through Shiny's credentials contract. Incoming
identity headers are stripped before credentials are set; a random worker secret
protects the R WebSocket connection. This is not OIDC, SAML, PAM, LDAP, MFA,
account lifecycle management or a polished logout flow. Use TLS.

Admin access uses a separate randomly generated token in `state_dir/admin.token`.
Admin data endpoints accept only `Authorization: Bearer <token>`. The token can be
printed by the explicit `admin-token` command. An unprotected health endpoint
reports only that the admin server is listening, not that all apps are ready.

## Authentication proxies and groups

An authenticating reverse proxy can provide single sign-on through its own
OIDC/SAML/LDAP integration. Configure the exact proxy transport addresses and
the headers it overwrites after authentication:

```toml
[auth_proxy]
user_header = "x-auth-username"
groups_header = "x-auth-groups"
trusted_peers = ["127.0.0.1/32", "::1/128"]

[[apps]]
name = "reports"
mount = "/reports"
path = "apps/reports"
allowed_users = ["*"]
allowed_groups = ["analysts", "administrators"]
```

Trust is based on the TCP peer, independently of `trusted_proxies` and forwarded
IP headers. Restrict network access to the proxy, and have it remove client-supplied
identity headers. On a shared machine, loopback access alone does not distinguish
other local users from the proxy. Remote proxies need explicit `trusted_peers`
and HTTPS for client-facing authentication (`public_origin` when TLS terminates
at the proxy). Proxy authentication takes precedence over local Basic passwords.

Groups are comma-separated, trimmed, deduplicated and matched case-sensitively.
An empty user list or `"**"` permits anonymous users; `"*"` requires an authenticated
user. A nonempty group list requires at least one matching group. When both user
and group restrictions are configured, both are enforced. Native Shiny receives
the verified identity in `session$user` and `session$groups`, including on public
apps. Session reconnects require the same user and group set as the original
connection. Duplicate identity headers are rejected; configured proxy headers
are removed before upstream forwarding.

The original directives `auth_proxy [userHeader [groupsHeader]]`, `required_user`
and `required_group` are also accepted, including inherited route restrictions.
Legacy `auth_proxy` trusts loopback peers only; use TOML for remote peer ranges.
Other professional identity providers and case-insensitive access rules are not
implemented natively. `tests/auth_proxy.py` covers the real R identity contract
and denial cases; it is not a licensed professional-edition comparison.

## Page templates

Set `template_dir = "templates"` globally, on a virtual server, or on an app/site's
app defaults. The equivalent `template_dir templates;` directive inherits through
root, server and location scopes. Paths are relative to the configuration file.
Custom `directoryIndex.html` receives `title`, `dirs` and `files`; each entry has
`name` and a percent-encoded `url`. App directories are included in `dirs`.
Custom errors use `error-<HTTP status>.html`, falling back to `error.html`, with
`title`, `message`, `detail`, `detailHTML` and `console` fields. Internal errors
remain sanitized; raw R console output is not exposed to visitors.

Templates use Handlebars, with normal HTML escaping unless explicitly overridden
by the template author. Missing files fall back to the built-in page. File changes
are recompiled without a server restart. Config checks reject invalid syntax;
files are limited to 1 MiB and rendered output to 8 MiB. This supports directory
and generic status-error templates; upstream's specialized error titles, messages,
helper functions and error subtypes have not all been compared.

## Worker logs

App settings `log_dir`, `log_file_mode = "0640"` and `preserve_logs = false` control
worker output. Original `log_dir`, `log_file_mode` and root `preserve_logs` directives
are accepted. Unix modes are applied without relying on the process umask; Windows
uses the directory's inherited ACLs. Each worker has separate stdout/stderr files,
rotated at 10 MiB with one backup. Clean exit removes its files unless preservation
is enabled; failed starts and unsuccessful exits retain diagnostic output.
Preserved files have a `.keep` marker and are excluded from age-based pruning.

Set the server's `SHINY_LOG_STDERR` environment variable to a nonempty value to
also forward worker output to server stderr. Preserved/custom logs require an
operator retention policy. Log files, configured log/bookmark directories, server
state, TLS private keys and the running configuration are excluded from static
site serving. Upstream's merged timestamp-based log filenames are not yet matched.

Set `log_as_user = true` (or inherited `log_as_user true;`) when logs must be created
and managed by the R account. With a changed Unix identity, a small logging helper
runs with that UID, GID and supplementary groups. Creation, rotation and deletion
all happen there; the gateway keeps its own credentials. Configure a `log_dir`
writable by that account. The helper has bounded input buffering and no runtime
thread pool. This addresses the ownership requirements of NFS root-squash setups;
the local tests verify Unix credentials and file operations, not an actual NFS mount.
On Windows, the service account manages its logs.

## Large Shiny messages and reconnect buffers

The native SockJS gateway accepts messages up to 64 MiB by default. Large plots,
table updates and custom messages can therefore exceed the previous 1 MiB limit.
Use a root TOML table to change the limits:

```toml
[sockjs]
max_message_bytes = 67108864
max_replay_bytes = 134217728
max_replay_messages = 2048
memory_budget_bytes = 536870912
```

`max_message_bytes` limits encoded WebSocket messages and HTTP send bodies,
including framing overhead. The replay settings bound unacknowledged messages
per logical session; they also configure the browser's send buffer, measured in
UTF-8 bytes. Acknowledgments release buffered data. `memory_budget_bytes` limits
queued/replay storage shared across gateway sessions; it is not a total process
memory limit and excludes transient encoding, network buffers and R workers.
These limits allocate memory only as messages arrive, not at startup.

Message limits accept 1 KiB through 1 GiB. Replay storage must exceed the message
limit by at least 1 KiB and cannot exceed 2 GiB. The shared budget must accommodate
the replay limit, two maximum messages and 4 KiB, and must be below 4 GiB.
Exhaustion closes the affected transport/session rather than growing without
bound. Limits are server-wide and changing them requires a restart. Legacy
`.conf` configurations use the defaults. Direct upstream proxy tunnels retain
their streaming behavior.

## R startup profiles and supervisors

On Unix, native `.conf` configurations start workers through `/bin/bash --login`.
Bash reads `/etc/profile`, then the first readable file among `.bash_profile`,
`.bash_login` and `.profile` in the R account's home. Credential changes happen
before Bash starts. The worker returns to the app directory after reading profiles.
TOML starts R directly by default; set `login_shell = true` on an app or in a site's
`app_defaults` to opt in. Windows uses the service environment and rejects an
explicit `login_shell = true`.

The inherited legacy directive `exec_supervisor "nice -n 10";` prefixes the R
command with a supervisor. An empty string clears an inherited supervisor. The
TOML equivalent is `supervisor = ["nice", "-n", "10"]`. Supervisors run under the
same account as R, and their descendants belong to the worker process group or
Windows Job Object. They must remain in the foreground and return the R exit
status. Executables must be available in the worker's environment.

Supervisor strings use POSIX argument quoting; shell expansion, operators and
redirection are not evaluated. Use a wrapper script when those are needed.
Arguments and app paths are passed literally, including spaces and metacharacters.
App-local `.shiny_app.conf` files cannot change the supervisor or login-shell policy.
Login profiles and supervisor scripts are trusted application-account code.

Before evaluating app code, the launcher sets `SHINY_PORT` to the worker's private
TCP port or Unix socket and fills `shiny::serverInfo()` with `shinyServer = TRUE`,
`edition = "ShinyHarbor"` and the actual ShinyHarbor version. The separate
`SHINY_SERVER_VERSION` / `protocolVersion` value `1.5.23` is the target runtime
protocol used by Shiny's hosted-environment version check. It is not a conformance
certificate or a claim that Posit's software is running. The shared secret and
bookmark callbacks still protect this hosted runtime.

## Unix worker sockets

Set root `socket_dir = "/run/shinyharbor/sockets"` in TOML or
`socket_dir /run/shinyharbor/sockets;` in legacy configuration to use Unix domain
sockets for HTTP and WebSocket traffic between the gateway and R. Each worker gets
a private directory and socket; its shared-secret check still applies. Socket paths
are limited to 103 bytes for Linux/macOS portability, so choose a short root. Normal
shutdown removes worker directories. Existing directory permissions are retained;
a new root allows account traversal while worker directories remain private.

Without `socket_dir`, workers use private loopback TCP on every platform. Windows
rejects an explicitly configured Unix `socket_dir` with an actionable error. Both
transports execute the same installed Shiny package and expose the same public routes.

## Framing and external analytics

Original `frame_options allow-from https://portal.example;` is accepted alongside
`allow`, `deny` and `sameorigin`. In TOML use
`frame_options = { allow_from = "https://portal.example" }`. The legacy
`ALLOW-FROM` header is preserved for compatibility; modern browsers may ignore it,
so it is not a substitute for an application Content Security Policy.

`google_analytics_id = "G-EXAMPLE"` or inherited
`google_analytics_id G-EXAMPLE;` adds Google tag markup to native Shiny HTML.
An empty string disables inherited tracking. This is an explicit external analytics
integration, separate from the local dashboard's aggregate collection and DNT/GPC
controls. It is off by default. The integration tests inspect HTML without loading
external scripts or contacting Google.

## HTTP access logging and inactivity

Access logging is optional and independent of privacy-preserving aggregate usage
statistics. To enable it:

```toml
http_keepalive_timeout_seconds = 45.0

[access_log]
path = "access.log"
format = "combined"
```

The original syntax is `access_log access.log combined;` and
`http_keepalive_timeout 45;`. Formats include `default`, `combined`, `common`,
`short`, `tiny`, and `dev` (without terminal color escapes), or a custom token
format. Supported tokens are `:remote-addr`, `:remote-user`, `:method`, `:url`,
`:http-version`, `:status`, `:referrer`, `:user-agent`, `:req[header]`, `:res[header]`,
`:date[web|clf|iso]`, `:response-time[digits]` and `:total-time[digits]`. Request
fields are escaped to keep one record per line. `remote-addr` is the transport
peer; `remote-user` is a presented Basic username, not proof of authentication.

Access logs contain raw addresses, URLs and selected headers; DNT/GPC affects the
aggregate statistics, not this explicitly enabled operational log. The default
formats never record passwords or authorization headers. The bounded asynchronous
writer reports lost records as `shinyharbor_dropped_access_logs` on the private
metrics endpoint. Rotate logs externally and run `shinyharbor reload` (or SIGHUP
on Unix) to reopen the path. Ensure the parent directory exists and is writable.

The inactivity timeout resets on socket reads or writes and applies to public
HTTP, HTTPS and upgraded connections. The default is 45 seconds; `0` disables it.
Fractional seconds are accepted. Set it above the SockJS heartbeat interval to
keep otherwise idle Shiny connections alive. Changing this setting requires a
service restart.
