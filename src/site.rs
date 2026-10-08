//! Request-time site discovery. App sources are never served as static files.
use crate::config::{App, Site, is_app, safe_name};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use http::{Method, Request, StatusCode, header};
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use tower_http::services::ServeFile;

pub enum Target {
    App(Box<App>),
    Static(PathBuf, Box<Site>),
}

pub fn private_paths(config: &crate::config::Config, file: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![config.state_dir.clone()];
    paths.extend(config.socket_dir.clone());
    paths.push(config.state_dir.with_extension("users"));
    paths.extend(file.map(Path::to_owned));
    paths.extend(config.access_log.iter().map(|log| log.path.clone()));
    if let crate::config::Tls::Pem { key, .. } = &config.tls {
        paths.push(key.clone());
    }
    for app in config
        .apps
        .iter()
        .chain(config.sites.iter().map(|s| &s.app_defaults))
    {
        paths.extend(app.log_dir.clone());
        paths.extend(app.bookmark_state_dir.clone());
    }
    paths.sort();
    paths.dedup();
    paths
}

async fn blocked(path: &Path, private: &[PathBuf]) -> bool {
    for item in private {
        if let Ok(item) = tokio::fs::canonicalize(item).await
            && path.starts_with(item)
        {
            return true;
        }
    }
    false
}

fn segments(path: &str) -> Result<Vec<String>, StatusCode> {
    if path.contains("//") {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Reject malformed escapes rather than decoding them inconsistently at R.
    let b = path.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%'
            && (i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit())
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    path.trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            let value = percent_decode_str(segment)
                .decode_utf8()
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .into_owned();
            if value.starts_with('.')
                || value.ends_with(['.', ' '])
                || value.contains(['/', '\\', ':'])
                || value.chars().any(char::is_control)
            {
                return Err(StatusCode::NOT_FOUND);
            }
            // Windows device names are not files, including when they have suffixes.
            let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
            if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(StatusCode::NOT_FOUND);
            }
            Ok(value)
        })
        .collect()
}

pub async fn resolve(site: &Site, path: &str, private: &[PathBuf]) -> Result<Target, StatusCode> {
    let relative = if site.mount == "/" {
        path
    } else {
        path.strip_prefix(&site.mount)
            .ok_or(StatusCode::NOT_FOUND)?
    };
    let parts = segments(relative)?;
    let mut site = site.clone();
    let mut parts = parts;
    let private = private.to_vec();
    tokio::task::spawn_blocking(move || {
        let private: Vec<_> = private
            .iter()
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();
        if site.user_dirs {
            if parts.is_empty() {
                return Err(StatusCode::NOT_FOUND);
            }
            let user = parts.remove(0);
            if !safe_name(&user) {
                return Err(StatusCode::NOT_FOUND);
            }
            site.path = crate::identity::home_if_member(&user, &site.app_defaults.members_of)
                .map_err(|_| StatusCode::NOT_FOUND)?
                .join("ShinyApps");
            site.mount = format!("{}/{}", site.mount.trim_end_matches('/'), user);
            for name in &mut site.app_defaults.run_as {
                if name == ":HOME_USER:" {
                    *name = user.clone();
                }
            }
            if site.app_defaults.log_dir.is_none() {
                site.app_defaults.log_dir = Some(site.path.join("log"));
            }
        }
        let root = std::fs::canonicalize(&site.path).map_err(|_| StatusCode::NOT_FOUND)?;
        let mut candidate = root.clone();
        for depth in 0..=parts.len() {
            let resolved = std::fs::canonicalize(&candidate).map_err(|_| StatusCode::NOT_FOUND)?;
            if !resolved.starts_with(&root) || private.iter().any(|p| resolved.starts_with(p)) {
                return Err(StatusCode::NOT_FOUND);
            }
            if site
                .app_defaults
                .log_dir
                .as_ref()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|logs| resolved.starts_with(logs))
            {
                return Err(StatusCode::NOT_FOUND);
            }
            let app_path = if is_app(&resolved) {
                Some(resolved.clone())
            } else if resolved.is_dir() && resolved.join("index.Rmd").is_file() {
                Some(resolved.join("index.Rmd"))
            } else if resolved.is_file() && is_rmarkdown(&resolved) {
                Some(resolved.clone())
            } else {
                None
            };
            if let Some(app_path) = app_path {
                let app_path =
                    std::fs::canonicalize(app_path).map_err(|_| StatusCode::NOT_FOUND)?;
                if !app_path.starts_with(&root) {
                    return Err(StatusCode::NOT_FOUND);
                }
                let suffix = parts[..depth]
                    .iter()
                    .map(|p| crate::config::encode_segment(p))
                    .collect::<Vec<_>>()
                    .join("/");
                let mount = format!("{}/{}", site.mount.trim_end_matches('/'), suffix)
                    .trim_end_matches('/')
                    .to_owned();
                let mount = if mount.is_empty() {
                    "/".to_owned()
                } else {
                    mount
                };
                let name = format!(
                    "site-{}",
                    hex::encode(&Sha256::digest(format!("{}:{mount}", site.name).as_bytes())[..12])
                );
                return Ok(Target::App(Box::new(App {
                    name,
                    label: Some(mount.clone()),
                    mount,
                    path: app_path,
                    ..site.app_defaults.clone()
                })));
            }
            if depth == parts.len() {
                site.path = root;
                return Ok(Target::Static(resolved, Box::new(site)));
            }
            candidate.push(&parts[depth]);
        }
        Err(StatusCode::NOT_FOUND)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}
fn is_rmarkdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("rmd"))
}
fn source_file(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        let lower = n.to_ascii_lowercase();
        n.starts_with('.')
            || lower == "restart.txt"
            || lower.ends_with(".log")
            || lower.ends_with(".log.1")
            || lower.ends_with(".log.keep")
    }) || path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "r" | "rmd" | "rdata" | "rds" | "qmd"
        )
    })
}
pub async fn serve(
    site: &Site,
    mut path: PathBuf,
    req: Request<Body>,
    private: &[PathBuf],
) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
        response
            .headers_mut()
            .insert(header::ALLOW, http::HeaderValue::from_static("GET, HEAD"));
        return response;
    }
    if path.is_dir() {
        if !req.uri().path().ends_with('/') {
            let query = req
                .uri()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default();
            return redirect(
                &format!("{}/{query}", req.uri().path()),
                StatusCode::MOVED_PERMANENTLY,
            );
        }
        let index = path.join("index.html");
        if index.is_file() {
            let Ok(canonical) = tokio::fs::canonicalize(index).await else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if !canonical.starts_with(&site.path) || blocked(&canonical, private).await {
                return StatusCode::NOT_FOUND.into_response();
            }
            path = canonical;
        } else if site.directory_index {
            return directory(
                &path,
                req.method() == Method::HEAD,
                site.app_defaults.template_dir.as_deref(),
                req.uri().path(),
                private,
            )
            .await;
        } else {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    if source_file(&path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    // ServeFile streams with bounded buffers and implements HEAD, range requests,
    // Last-Modified / If-Modified-Since, Content-Length and MIME handling.
    match ServeFile::new(path).oneshot(req).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}
pub fn redirect(target: &str, status: StatusCode) -> Response {
    match target.parse::<http::HeaderValue>() {
        Ok(value) => {
            let mut r = status.into_response();
            r.headers_mut().insert(header::LOCATION, value);
            r
        }
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}
async fn directory(
    path: &Path,
    head: bool,
    template: Option<&Path>,
    url: &str,
    private: &[PathBuf],
) -> Response {
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut names = Vec::new();
    let mut hidden = Vec::new();
    for path in private {
        if let Ok(path) = tokio::fs::canonicalize(path).await {
            hidden.push(path);
        }
    }
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if segments(&crate::config::encode_segment(&name)).is_err()
            || (source_file(&entry.path()) && !is_rmarkdown(&entry.path()))
            || hidden.iter().any(|p| entry.path().starts_with(p))
        {
            continue;
        }
        // Hide links/junctions; requested targets are checked separately.
        if let Ok(meta) = entry.file_type().await
            && !meta.is_symlink()
        {
            names.push((name, meta.is_dir()));
        }
        if names.len() > 10_000 {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    }
    names.sort_by_cached_key(|(name, _)| name.to_lowercase());
    let mut html = String::from(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Applications and files</title><h1>Applications and files</h1><ul>",
    );
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for (name, dir) in names {
        let slash = if dir { "/" } else { "" };
        let href = crate::config::encode_segment(&name);
        let row = serde_json::json!({"name":name,"url":href});
        if dir {
            dirs.push(row);
        } else {
            files.push(row);
        }
        let label = name
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        html.push_str(&format!(
            "<li><a href=\"{href}{slash}\">{label}{slash}</a></li>"
        ));
    }
    html.push_str("</ul></html>");
    if let Some(template) = template {
        let data = serde_json::json!({"title":format!("Index of {}", percent_decode_str(url).decode_utf8_lossy()), "dirs":dirs,"files":files});
        match crate::templates::render(template, "directoryIndex", data).await {
            Ok(Some(rendered)) => html = rendered,
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "Cannot render custom directory template");
                return crate::proxy::problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Directory template could not be rendered",
                );
            }
        }
    }
    let length = html.len();
    let mut response = axum::response::Html(html).into_response();
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        length
            .to_string()
            .parse()
            .expect("A length is a valid header"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    if head {
        *response.body_mut() = Body::empty();
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_paths_are_rejected_on_every_os() {
        for path in [
            "/../x",
            "/%2e%2e/x",
            "/a%2fb",
            "/a%5cb",
            "/.env",
            "/x:stream",
            "/NUL.txt",
            "/x.",
            "/x%00",
            "/%zz",
            "/%",
            "/%ff",
            "//host/path",
        ] {
            assert!(segments(path).is_err(), "{path}");
        }
        assert_eq!(segments("/a%20b/file.txt").unwrap(), ["a b", "file.txt"]);
    }
    #[tokio::test]
    async fn apps_added_after_startup_are_discovered_without_serving_sources() {
        let temp = tempfile::tempdir().unwrap();
        let site = Site {
            user_dirs: false,
            name: "test".into(),
            mount: "/reports".into(),
            path: temp.path().to_owned(),
            directory_index: false,
            app_defaults: App::default(),
        };
        assert!(resolve(&site, "/reports/new/", &[]).await.is_err());
        std::fs::create_dir(temp.path().join("new")).unwrap();
        std::fs::write(temp.path().join("new/app.R"), "private R source").unwrap();
        let Target::App(app) = resolve(&site, "/reports/new/app.R", &[]).await.unwrap() else {
            panic!("App source exposed as static file")
        };
        assert_eq!(app.mount, "/reports/new");
        assert_eq!(app.path.file_name().unwrap(), "new");
    }
    #[tokio::test]
    async fn static_ranges_head_and_cache_validation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.txt");
        std::fs::write(&path, "0123456789").unwrap();
        let site = Site {
            user_dirs: false,
            name: "test".into(),
            mount: "/".into(),
            path: temp.path().to_owned(),
            directory_index: false,
            app_defaults: App::default(),
        };
        let request = Request::builder()
            .uri("/data.txt")
            .header(header::RANGE, "bytes=2-5")
            .body(Body::empty())
            .unwrap();
        let response = serve(&site, path.clone(), request, &[]).await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "2345"
        );
        let response = serve(
            &site,
            path.clone(),
            Request::builder()
                .method(Method::HEAD)
                .uri("/data.txt")
                .body(Body::empty())
                .unwrap(),
            &[],
        )
        .await;
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
        let modified = response.headers()[header::LAST_MODIFIED].clone();
        assert!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap()
                .is_empty()
        );
        let request = Request::builder()
            .uri("/data.txt")
            .header(header::IF_MODIFIED_SINCE, modified)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            serve(&site, path, request, &[]).await.status(),
            StatusCode::NOT_MODIFIED
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_cannot_escape_site_root() {
        let site_dir = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::write(private.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(private.path(), site_dir.path().join("link")).unwrap();
        let site = Site {
            user_dirs: false,
            name: "test".into(),
            mount: "/".into(),
            path: site_dir.path().to_owned(),
            directory_index: false,
            app_defaults: App::default(),
        };
        assert!(resolve(&site, "/link/secret.txt", &[]).await.is_err());
    }
}
