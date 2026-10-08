Alpha source implementation for evaluation. Not a certified Shiny Server replacement.

Includes a Rust HTTP/WebSocket gateway, native R worker pools, integrated TLS/ACME,
a loopback-only analytics dashboard, configuration/CLI helpers and service templates.
Original Shiny Server remains an optional independently installed upstream.

The complete zero-compatibility-gap requirement and Professional feature parity are
not met. Read COMPATIBILITY.md and SECURITY.md before use. A successful prerelease
workflow demonstrates only the checks actually present in that workflow; it does
not certify all behavior in the outstanding conformance inventory.

Each binary archive includes dependency notices and the resolved lockfile. R,
application packages, upstream Shiny Server and country databases are not bundled.
