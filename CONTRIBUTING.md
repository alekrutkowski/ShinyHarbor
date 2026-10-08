# Contributing

Use a current stable Rust toolchain and a C compiler. Run `cargo fmt --all`,
`cargo clippy --all-targets`, `cargo test --all-targets`, the Python structural
checks, gateway integration tests against the built executable, and browser tests.
R changes need real package/application checks, not only compilation.

Do not weaken the stable release gate or describe smoke checks as full Shiny Server
compatibility. Add a fixture, an exact reference version, a defined expected outcome
and retained evidence for each compatibility item. Test unhappy paths and platform
lifecycle behavior, not just the happy-path plot example.

Keep app data, credentials, generated state, country databases and real usage logs
out of commits. Review dependency licenses/advisories and retain their notices.
Contributions are under the repository's MIT license unless explicitly agreed otherwise.
