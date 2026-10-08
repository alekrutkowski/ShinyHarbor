# Primary-source references checked during preparation

Reference date: 2026-10-02. These document upstream behavior and dependency APIs,
not a claim that this project's integration has passed its tests.

- CRAN Shiny stable metadata (1.14.0, published 2026-06-21): https://cran.r-project.org/web/packages/shiny/index.html
- Posit Shiny Server administration and directives: https://docs.posit.co/shiny-server/
- Shiny Server source/license: https://github.com/rstudio/shiny-server
- Shiny Server current development configuration rules: https://raw.githubusercontent.com/rstudio/shiny-server/master/config/shiny-server-rules.config
- Shiny R package source: https://github.com/rstudio/shiny
- axum: https://docs.rs/axum/latest/axum/
- axum-server from_tcp API: https://docs.rs/axum-server/latest/axum_server/fn.from_tcp.html
- rustls-acme integration: https://github.com/FlorianUekermann/rustls-acme
- Let's Encrypt challenge types: https://letsencrypt.org/docs/challenge-types/
- process-wrap child/group/job APIs: https://github.com/watchexec/process-wrap
- MaxMind DB reader API: https://docs.rs/maxminddb/latest/maxminddb/
- Windows service SCM API: https://github.com/mullvad/windows-service-rs
- GitHub runner labels/platforms: https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- Windows ARM64 runner inventory: https://github.com/actions/runner-images/blob/main/images/windows/Windows11-Arm64-Readme.md

Development branch documentation can diverge from a published release. Resolve and
record actual crate versions with Cargo.lock and exact upstream commits for any
conformance result. The package's current stable version is not its development
DESCRIPTION version, and a Rust binary target is not an R runtime support claim.
