use crate::{
    analytics::Analytics,
    auth::Auth,
    config::{Config, Tls},
    pool::Pool,
    proxy::{self, AppState},
    security::{self, Secret},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs::File,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tower_http::compression::{
    CompressionLayer,
    predicate::{DefaultPredicate, Predicate},
};

pub fn admin_token(state: &Path) -> Result<String> {
    security::private_dir(state)?;
    let path = state.join("admin.token");
    match security::private_create(&path) {
        Ok(mut file) => {
            let token = format!("{}{}", security::random_id(), security::random_id());
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(&path)?;
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Admin token must be a regular non-symlink file"
            );
            let mut token = String::new();
            File::open(path)?.take(129).read_to_string(&mut token)?;
            let token = token.trim().to_owned();
            ensure!(
                (40..=128).contains(&token.len())
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "Invalid admin token file"
            );
            Ok(token)
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn run(config: Config, stop: CancellationToken) -> Result<()> {
    run_config(config, None, stop).await
}
pub async fn run_file(path: &Path, stop: CancellationToken) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    run_config(Config::load(&path)?, Some(path), stop).await
}
async fn run_config(config: Config, path: Option<PathBuf>, stop: CancellationToken) -> Result<()> {
    // The state lock covers keys, readiness files and the SQLite writer.
    security::private_dir(&config.state_dir)?;
    let lock_path = config.state_dir.join("instance.lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("Another ShinyHarbor process is using this state directory")?;
    for name in ["run", "logs", "acme"] {
        security::private_dir(&config.state_dir.join(name))?;
    }
    if let Some(directory) = &config.socket_dir {
        if !directory.exists() {
            std::fs::create_dir_all(directory)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o711))?;
            }
        }
        ensure!(
            std::fs::symlink_metadata(directory)?.is_dir(),
            "socket_dir must be a real directory"
        );
    }
    let bootstrap = config.state_dir.join("bootstrap.R");
    if bootstrap.exists() {
        ensure!(
            !std::fs::symlink_metadata(&bootstrap)?
                .file_type()
                .is_symlink(),
            "bootstrap.R must not be a symlink"
        );
    }
    std::fs::write(bootstrap, include_str!("../r/bootstrap.R"))?;
    let runtime = config.state_dir.join("runtime.R");
    if runtime.exists() {
        ensure!(
            !std::fs::symlink_metadata(&runtime)?
                .file_type()
                .is_symlink(),
            "runtime.R must not be a symlink"
        );
    }
    std::fs::write(runtime, include_str!("../r/runtime.R"))?;
    let secret = Secret::load_or_create(&config.state_dir.join("secret.key"))?;
    let token = admin_token(&config.state_dir)?;
    // Bind both sockets before starting any application. Do not leave half a server running.
    let mut public_listeners = Vec::new();
    for address in config.listeners() {
        let listener =
            TcpListener::bind(address).with_context(|| format!("Cannot bind {address}"))?;
        listener.set_nonblocking(true)?;
        public_listeners.push((address, listener));
    }
    let admin_listener = TcpListener::bind(config.admin_listen)
        .with_context(|| format!("Cannot bind admin {}", config.admin_listen))?;
    admin_listener.set_nonblocking(true)?;
    let redirect_listener = config
        .redirect_listener()
        .map(|address| -> Result<_> {
            let listener = TcpListener::bind(address)
                .with_context(|| format!("Cannot bind HTTP redirect listener {address}"))?;
            listener.set_nonblocking(true)?;
            Ok(listener)
        })
        .transpose()?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pem = match &config.tls {
        Tls::Pem { cert, key } => Some(
            axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .context("Cannot load TLS certificate/key")?,
        ),
        _ => None,
    };
    let analytics = Analytics::start(
        &config.analytics,
        &config.state_dir.join("analytics.sqlite"),
        secret.clone(),
    )?;
    let access_log = match &config.access_log {
        Some(log) => Some(
            crate::access_log::AccessLog::start(log)
                .await
                .context("Cannot open access log")?,
        ),
        None => None,
    };
    let mut connector = HttpConnector::new();
    connector.set_connect_timeout(Some(Duration::from_secs(5)));
    connector.set_nodelay(true);
    let client = Client::builder(TokioExecutor::new())
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .build::<_, Body>(connector);
    let startup = Arc::new(Semaphore::new(4));
    let log_tasks = TaskTracker::new();
    let mut pools: Vec<_> = config
        .apps
        .iter()
        .cloned()
        .map(|a| Arc::new(Pool::native(a, &config, startup.clone(), log_tasks.clone())))
        .chain(
            config
                .upstreams
                .iter()
                .cloned()
                .map(|u| Arc::new(Pool::upstream(u))),
        )
        .collect();
    pools.sort_by_key(|p| std::cmp::Reverse(p.mount().len()));
    let state = Arc::new(AppState {
        private_paths: Arc::new(crate::site::private_paths(&config, path.as_deref())),
        access_log,
        templates_enabled: config.uses_templates(),
        auth: Auth::new(config.users.clone(), secret.clone()).with_proxy(config.auth_proxy.clone()),
        slots: Arc::new(Semaphore::new(config.max_inflight)),
        config: Arc::new(config),
        pools: std::sync::RwLock::new(pools),
        startup,
        log_tasks,
        client,
        #[cfg(unix)]
        socket_client: Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .build(hyperlocal::UnixConnector),
        analytics,
        secret,
        admin_token: token,
        force_stop: CancellationToken::new(),
        tunnels: TaskTracker::new(),
        sockjs: Arc::new(crate::sockjs::Hub::default()),
        control: std::sync::OnceLock::new(),
    });
    let control = crate::reload::Control::new(state.clone(), path);
    let app = Router::new()
        .fallback(proxy::handle)
        .with_state(control.clone());
    let app = if state.config.http_allow_compression {
        app.layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(compressible)))
    } else {
        app
    };
    let app = if let Some(log) = &state.access_log {
        app.layer(axum::middleware::from_fn_with_state(
            log.clone(),
            crate::access_log::middleware,
        ))
    } else {
        app
    };
    let admin = Router::new()
        .route("/", get(dashboard))
        .route("/app.js", get(javascript))
        .route("/style.css", get(stylesheet))
        .route("/healthz", get(health))
        .route("/api/stats", get(stats))
        .route("/api/status", get(status))
        .route("/api/reload", post(reload))
        .route("/metrics", get(metrics))
        .with_state(control.clone());
    let public_handles: Vec<_> = public_listeners
        .iter()
        .map(|_| axum_server::Handle::new())
        .collect();
    let admin_handle = axum_server::Handle::new();
    let public_handles2 = public_handles.clone();
    let tls = state.config.tls.clone();
    let idle_timeout = Duration::from_secs_f64(state.config.http_keepalive_timeout_seconds);
    let cache = state.config.state_dir.join("acme");
    let background_stop = CancellationToken::new();
    let pem_reload = match (&state.config.tls, pem.clone()) {
        (Tls::Pem { cert, key }, Some(pem)) => Some(tokio::spawn(crate::tls::reload_pem(
            pem,
            cert.clone(),
            key.clone(),
            background_stop.clone(),
        ))),
        _ => None,
    };
    let redirect_handle = axum_server::Handle::new();
    let redirect_handle2 = redirect_handle.clone();
    let redirect_config = state.config.clone();
    let acme_stop = background_stop.clone();
    let mut public_task = tokio::spawn(async move {
        let mut listeners = tokio::task::JoinSet::new();
        if let Some(listener) = redirect_listener {
            listeners.spawn(async move {
                let router = Router::new()
                    .fallback(crate::tls::redirect)
                    .with_state(redirect_config);
                axum_server::from_tcp(listener)?
                    .handle(redirect_handle2)
                    .serve(router.into_make_service())
                    .await?;
                Ok::<_, anyhow::Error>(())
            });
        }
        for ((address, public_listener), public_handle2) in
            public_listeners.into_iter().zip(public_handles2)
        {
            let app = app.clone().layer(axum::Extension(address));
            let tls = tls.clone();
            let pem = pem.clone();
            let cache = cache.clone();
            let acme_stop = acme_stop.clone();
            listeners.spawn(async move {
        match tls {
            Tls::Off {} => axum_server::from_tcp(public_listener)?.map(|a| crate::connection::IdleAcceptor::new(a, idle_timeout)).handle(public_handle2)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?,
            Tls::Pem { .. } => axum_server::from_tcp_rustls(public_listener, pem.context("Missing PEM configuration")?)?
                .map(|a| crate::connection::IdleAcceptor::new(a, idle_timeout))
                .handle(public_handle2).serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?,
            Tls::Acme { .. } => {
                let mut acme = crate::tls::acme(&tls, &cache)?.state();
                let acceptor = acme.axum_acceptor(acme.default_rustls_config());
                let renew = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = acme_stop.cancelled() => break,
                            next = acme.next() => match next {
                                Some(Ok(event)) => tracing::info!(?event, "ACME certificate event"),
                                Some(Err(error)) => tracing::error!(?error, "ACME error; check DNS, port 443 and persistent cache"),
                                None => break,
                            }
                        }
                    }
                });
                let result = axum_server::from_tcp(public_listener)?.handle(public_handle2).acceptor(acceptor)
                    .map(|a| crate::connection::IdleAcceptor::new(a, idle_timeout))
                    .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await;
                renew.abort(); let _ = renew.await;
                result?;
            }
        }
        Ok::<_, anyhow::Error>(())
        });
        }
        while let Some(result) = listeners.join_next().await {
            result??;
        }
        Ok::<_, anyhow::Error>(())
    });
    let admin_handle2 = admin_handle.clone();
    let mut admin_task = tokio::spawn(async move {
        axum_server::from_tcp(admin_listener)?
            .handle(admin_handle2)
            .serve(admin.into_make_service())
            .await
    });
    let reaper_stop = background_stop.clone();
    let reaper_control = control.clone();
    let reaper = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut rounds = 0_u64;
        loop {
            tokio::select! {
                _ = reaper_stop.cancelled() => break,
                _ = interval.tick() => {
                    for pool in reaper_control.pools() { pool.reap().await; }
                    reaper_control.prune_retired();
                    let reaper_state = reaper_control.snapshot();
                    reaper_state.sockjs.reap(Duration::from_secs_f64(reaper_state.config.sockjs_disconnect_seconds));
                    rounds += 1;
                    if rounds.is_multiple_of(720) {
                        let path = reaper_state.config.state_dir.join("logs");
                        let days = reaper_state.config.analytics.retention_days;
                        let _ = tokio::task::spawn_blocking(move || prune_old_logs(&path, days)).await;
                    }
                }
            }
        }
    });
    #[cfg(unix)]
    let reload_signal = {
        let control = control.clone();
        let stop = background_stop.clone();
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    message = signal.recv() => {
                        if message.is_none() { break; }
                        if let Err(error) = control.reload().await { tracing::error!(%error, "Configuration reload rejected; previous configuration retained"); }
                    }
                }
            }
        })
    };
    tracing::info!(listen = %state.config.listen, admin = %state.config.admin_listen,
        "ShinyHarbor started. Use `shinyharbor admin-token --config <path>` to unlock the local dashboard");
    // Retain old routing snapshots only while requests or sessions use them.
    drop(state);
    let mut public_finished = false;
    let mut admin_finished = false;
    let outcome: Result<()> = tokio::select! {
        _ = stop.cancelled() => Ok(()),
        result = &mut public_task => { public_finished = true; result.map_err(anyhow::Error::from).and_then(|r| r) },
        result = &mut admin_task => { admin_finished = true; result.map_err(anyhow::Error::from).and_then(|r| r.map_err(anyhow::Error::from)) },
    };
    let state = control.snapshot();
    let grace = Duration::from_secs(state.config.shutdown_grace_seconds);
    for handle in &public_handles {
        handle.graceful_shutdown(Some(grace));
    }
    admin_handle.graceful_shutdown(Some(Duration::from_secs(2)));
    redirect_handle.graceful_shutdown(Some(Duration::from_secs(2)));
    background_stop.cancel();
    #[cfg(unix)]
    {
        let _ = reload_signal.await;
    }
    if let Some(task) = pem_reload {
        let _ = task.await;
    }
    // Upgraded connections are no longer owned by hyper; drain them explicitly.
    state.tunnels.close();
    let _ = tokio::time::timeout(grace, state.tunnels.wait()).await;
    state.force_stop.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), state.tunnels.wait()).await;
    if !public_finished
        && tokio::time::timeout(grace + Duration::from_secs(1), &mut public_task)
            .await
            .is_err()
    {
        public_task.abort();
    }
    // Admin has already been asked to drain. Abort only if its task is still pending.
    if !admin_finished {
        admin_task.abort();
        let _ = admin_task.await;
    }
    reaper.abort();
    let _ = reaper.await;
    for pool in control.pools() {
        pool.stop().await;
    }
    state.log_tasks.close();
    let _ = tokio::time::timeout(Duration::from_secs(5), state.log_tasks.wait()).await;
    state.analytics.shutdown().await;
    if let Some(log) = &state.access_log {
        log.shutdown().await;
    }
    drop(lock);
    outcome
}
fn prune_old_logs(path: &Path, days: u32) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let unrotated = if p.extension().is_some_and(|e| e == "1") {
            p.with_extension("")
        } else {
            p.clone()
        };
        if unrotated.with_extension("log.keep").exists() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".log") || n.ends_with(".log.1"))
            && meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > u64::from(days) * 86400)
        {
            let _ = std::fs::remove_file(p);
        }
    }
}
fn compressible(
    status: StatusCode,
    _: http::Version,
    headers: &HeaderMap,
    _: &http::Extensions,
) -> bool {
    status == StatusCode::OK
        && !headers
            .get_all(header::CACHE_CONTROL)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|directive| directive.trim().eq_ignore_ascii_case("no-transform"))
}
fn authorized(headers: &HeaderMap, state: &AppState) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .is_some_and(|token| security::constant_eq(token, &state.admin_token))
}
fn admin_error() -> Response {
    proxy::problem(
        StatusCode::UNAUTHORIZED,
        "A valid admin bearer token is required",
    )
}
fn secure_asset(content: &'static str, mime: &'static str) -> Response {
    let mut response = content.into_response();
    let h = response.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
    response
}
async fn dashboard() -> Response {
    secure_asset(
        include_str!("../assets/index.html"),
        "text/html; charset=utf-8",
    )
}
async fn javascript() -> Response {
    secure_asset(
        include_str!("../assets/app.js"),
        "text/javascript; charset=utf-8",
    )
}
async fn stylesheet() -> Response {
    secure_asset(
        include_str!("../assets/style.css"),
        "text/css; charset=utf-8",
    )
}
async fn health() -> Response {
    axum::Json(json!({"status":"listening", "version":env!("CARGO_PKG_VERSION")})).into_response()
}
async fn reload(
    State(control): State<Arc<crate::reload::Control>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &control.snapshot()) {
        return admin_error();
    }
    match control.reload().await {
        Ok(()) => axum::Json(json!({"reloaded": true})).into_response(),
        Err(error) => {
            tracing::warn!(%error, "Configuration reload rejected; previous configuration retained");
            proxy::problem(
                StatusCode::BAD_REQUEST,
                "Reload rejected. Check the server log; the previous configuration remains active.",
            )
        }
    }
}
#[derive(Deserialize)]
struct Period {
    days: Option<u32>,
}
async fn stats(
    State(control): State<Arc<crate::reload::Control>>,
    headers: HeaderMap,
    Query(period): Query<Period>,
) -> Response {
    let state = control.snapshot();
    if !authorized(&headers, &state) {
        return admin_error();
    }
    match state.analytics.snapshot(period.days.unwrap_or(7)).await {
        Ok(mut snapshot) => {
            let mut pools = Vec::new();
            for pool in state.pools() {
                pools.push(pool.status().await);
            }
            snapshot["pools"] = serde_json::to_value(pools).unwrap_or_default();
            let mut response = axum::Json(snapshot).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(error) => {
            tracing::warn!(%error, "Analytics query failed");
            proxy::problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Analytics is busy or unavailable",
            )
        }
    }
}
async fn status(
    State(control): State<Arc<crate::reload::Control>>,
    headers: HeaderMap,
) -> Response {
    let state = control.snapshot();
    if !authorized(&headers, &state) {
        return admin_error();
    }
    let mut pools = Vec::new();
    for pool in state.pools() {
        pools.push(pool.status().await);
    }
    let mut response = axum::Json(json!({"version":env!("CARGO_PKG_VERSION"), "pools":pools,
        "compatibility":"not-certified", "inflight":state.config.max_inflight - state.slots.available_permits()})).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
async fn metrics(
    State(control): State<Arc<crate::reload::Control>>,
    headers: HeaderMap,
) -> Response {
    let state = control.snapshot();
    if !authorized(&headers, &state) {
        return admin_error();
    }
    let mut out = String::from(
        "# HELP shinyharbor_active_websockets Current proxied WebSocket connections.\n# TYPE shinyharbor_active_websockets gauge\n",
    );
    for pool in state.pools() {
        let p = pool.status().await;
        out.push_str(&format!("shinyharbor_active_websockets{{app=\"{}\"}} {}\nshinyharbor_workers{{app=\"{}\"}} {}\n", p.name, p.active_websockets, p.name, p.workers));
    }
    out.push_str(&format!("# TYPE shinyharbor_dropped_analytics_events counter\nshinyharbor_dropped_analytics_events {}\n", state.analytics.dropped.load(std::sync::atomic::Ordering::Relaxed)));
    if let Some(log) = &state.access_log {
        out.push_str(&format!(
            "# TYPE shinyharbor_dropped_access_logs counter\nshinyharbor_dropped_access_logs {}\n",
            log.dropped.load(std::sync::atomic::Ordering::Relaxed)
        ));
    }
    let mut response = out.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
