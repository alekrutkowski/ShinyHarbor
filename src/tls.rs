//! HTTPS setup, private ACME trust roots, and safe canonical redirects.
use crate::config::{Config, Tls};
use anyhow::{Context, Result, ensure};
use axum::{
    body::Body,
    extract::State,
    response::{IntoResponse, Response},
};
use http::{Request, StatusCode, header};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use std::{path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub fn acme(config: &Tls, cache: &Path) -> Result<rustls_acme::AcmeConfig<std::io::Error>> {
    let Tls::Acme {
        domains,
        email,
        production,
        directory_url,
        ca_bundle,
    } = config
    else {
        anyhow::bail!("Not an ACME configuration");
    };
    let mut acme = rustls_acme::AcmeConfig::new(domains)
        .contact([format!("mailto:{email}")])
        .directory_lets_encrypt(*production)
        .cache(rustls_acme::caches::DirCache::new(cache.to_owned()));
    if let Some(directory) = directory_url {
        acme = acme.directory(directory);
    }
    if let Some(path) = ca_bundle {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_file_iter(path).context("Cannot read ACME CA bundle")? {
            roots.add(cert?)?;
        }
        ensure!(!roots.is_empty(), "ACME CA bundle is empty");
        acme = acme.client_tls_config(Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ));
    }
    Ok(acme)
}

pub async fn redirect(State(config): State<Arc<Config>>, request: Request<Body>) -> Response {
    let origin = if let Some(origin) = &config.public_origin {
        origin.trim_end_matches('/').to_owned()
    } else if let Tls::Acme { domains, .. } = &config.tls {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.parse::<http::uri::Authority>().ok());
        let Some(domain) = host
            .as_ref()
            .and_then(|h| domains.iter().find(|d| d.eq_ignore_ascii_case(h.host())))
        else {
            return StatusCode::MISDIRECTED_REQUEST.into_response();
        };
        let port = config.listen.port();
        if port == 443 {
            format!("https://{domain}")
        } else {
            format!("https://{domain}:{port}")
        }
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let target = format!(
        "{origin}{}",
        request.uri().path_and_query().map_or("/", |p| p.as_str())
    );
    crate::site::redirect(&target, StatusCode::PERMANENT_REDIRECT)
}

/// Replacement PEM files take effect without dropping existing TLS sessions.
/// An incomplete/invalid replacement leaves the last valid certificate in use.
pub async fn reload_pem(
    config: axum_server::tls_rustls::RustlsConfig,
    cert: std::path::PathBuf,
    key: std::path::PathBuf,
    stop: CancellationToken,
) {
    let stamp = || {
        [
            std::fs::metadata(&cert).and_then(|m| m.modified()).ok(),
            std::fs::metadata(&key).and_then(|m| m.modified()).ok(),
        ]
    };
    let mut previous = stamp();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = interval.tick() => {
                let current = stamp();
                if current != previous {
                    match config.reload_from_pem_file(&cert, &key).await {
                        Ok(()) => { previous = current; tracing::info!("TLS certificate files reloaded"); },
                        Err(error) => tracing::warn!(%error, "Invalid replacement TLS certificate; retaining previous certificate"),
                    }
                }
            }
        }
    }
}
