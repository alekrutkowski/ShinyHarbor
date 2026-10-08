//! ShinyHarbor's transport, scheduling and observability components.
//! Compatibility with Posit's server is a conformance property, not inferred
//! from the ability to start shiny::runApp(). See docs/COMPATIBILITY.md.
pub mod access_log;
mod activity;
pub mod analytics;
pub mod auth;
mod backend;
pub mod config;
mod connection;
pub mod identity;
pub mod legacy;
pub mod pool;
pub mod proxy;
pub mod reload;
pub mod security;
pub mod server;
pub mod site;
pub mod sockjs;
pub mod templates;
pub mod tls;
pub mod user_log;
