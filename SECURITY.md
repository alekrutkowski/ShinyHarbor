# Security and threat model

**Alpha source implementation. No independent security audit or production-hardening
claim is made. Do not expose this prototype directly to hostile Internet traffic.**
The complete compatibility/acceptance requirement is not satisfied.

## Trust boundary

Applications and their R startup files are trusted executable code. Native workers
run under the server account, not a separate per-app UID or security sandbox. They
can access whatever files, network destinations and secrets that account can
access, including state-directory contents when filesystem permissions allow it.
Do not host mutually untrusted tenants under one instance. OS containers or separate
accounts/instances must enforce tenant boundaries. Process groups and Windows Job
Objects provide lifecycle management, not security isolation or memory quotas.

Native R listens on loopback. Other local processes may still reach its ports and
bypass gateway authentication. Protect the host and isolate workloads. HTTP upstream
traffic must remain on a trusted local/private network. No encrypted upstream
transport is supplied. Original Shiny Server edition limits remain in upstream mode.

## Credentials and administration

The admin listener is restricted to loopback and its data endpoints require a
random bearer token. Access it through an authenticated SSH tunnel; do not expose
it with a blind port forward. Tokens are not accepted as query strings, persisted
in browser storage, or intentionally logged. The dashboard serves no remote scripts,
uses a restrictive CSP and constructs text through DOM APIs rather than `innerHTML`.
These controls are not a substitute for an audit.

Basic authentication is optional and separate from admin access. Password hashes
use Argon2. Hashing concurrency is bounded, but a comprehensive rate-limit/brute-force
policy and uniform unknown-user timing are not implemented. Basic credentials may
be retained by browsers until the browsing context closes. No MFA or SSO is included.
Do not claim `session$user` or proxy-auth parity from an `X-Harbor-User` header.

Forwarded IPs are accepted only from configured trusted CIDRs and interpreted
right-to-left. Broadly trusting `0.0.0.0/0` defeats that boundary. Standard forwarding
and selected identity headers are removed/replaced before proxying. Do not add
arbitrary identity pass-through to fix an app without designing its trust boundary.

## Files and privacy

On Unix, newly created secrets use mode 0600 and state directories use mode 0700.
Windows relies on inherited ACLs for ordinary CLI use; protect the state directory
explicitly, as in the service installer. Existing secret permissions, all ancestor
symlinks and hostile-local-user filesystem races are not comprehensively audited.
Run from administrator-controlled configuration and filesystem locations.

ACME account keys, TLS keys, the affinity signing key and admin token are secrets.
Persist them with restricted access. Losing/replacing the signing key invalidates
worker affinity. Any account able to read the key can forge routing cookies; these
are not authorization tokens, but should still be protected.

The analytics database contains hourly traffic buckets and daily keyed IP estimates.
These are pseudonymous, not guaranteed anonymous. Keeping the master key makes
brute-force comparisons against candidate IPs possible. No raw IP, full URL query,
request body or full referrer path is deliberately stored in analytics. R apps,
reverse proxies and application logs may independently record sensitive data.
Country estimates can be wrong. DNT/GPC headers suppress analytics events. Disabling
collection does not erase previously retained data. No GDPR or other legal
compliance guarantee is made.

## Availability and resource limits

Proxy requests/tunnels, startup concurrency, password hashing, event queue capacity
and worker counts are bounded. Bodies and WebSockets stream with backpressure
instead of being buffered in full. SQLite updates run on a dedicated thread.
These design choices have not been benchmarked here.

`max_inflight` does not bound all accepted idle TCP sockets or slow clients before
request dispatch. No complete slowloris defense, per-IP quota, application memory
quota, global log disk quota, or cardinality quota for referrer aggregates is provided.
Use an audited edge proxy and OS limits for public deployment only after further
review. Worker log files rotate at 10 MiB plus one backup; old-worker files accumulate
until age-based cleanup. Service logging has separate retention needs.

Failed proxy requests are not retried automatically, avoiding duplicate POST effects.
This does not preserve active sessions after an R crash or upstream failure.
WebSocket byte counts are incomplete for interrupted copy operations. Under load,
analytics can deliberately drop events and show an explicit count.

## Supply chain and reports

No Rust lockfile was fabricated. CI resolves one shared lockfile when absent and
packages it with releases. Before production/stable use, review and commit that
lockfile, pin the Rust toolchain and action SHAs, run dependency advisories, and
review transitive license notices. Actions currently use recognizable version tags;
these are not immutable provenance guarantees. Checksums detect corruption, not
malicious replacement of both an artifact and its checksum. Binaries are not
code-signed or notarized in these workflows.

Report security issues privately to Alek Rutkowski <alek.rutkowski@gmail.com>.
Avoid including production secrets or identifiable traffic in reports.
