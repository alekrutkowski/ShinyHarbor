use crate::{
    analytics::{Analytics, Event},
    auth::Auth,
    config::{Config, Tls, matches_mount},
    pool::{Lease, Pool},
    security::{self, Secret},
};
use axum::{
    Extension,
    body::Body,
    extract::{ConnectInfo, State},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use http::{HeaderValue, Method, Request, StatusCode, Uri, header};
use http_body::{Body as HttpBody, Frame, SizeHint};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioIo,
};
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, OnceLock, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub struct AppState {
    pub config: Arc<Config>,
    pub response_headers: http::HeaderMap,
    pub templates_enabled: bool,
    pub access_log: Option<Arc<crate::access_log::AccessLog>>,
    pub private_paths: Arc<Vec<std::path::PathBuf>>,
    pub log_tasks: TaskTracker,
    pub pools: RwLock<Vec<Arc<Pool>>>,
    pub startup: Arc<Semaphore>,
    pub client: Client<HttpConnector, Body>,
    #[cfg(unix)]
    pub socket_client: Client<hyperlocal::UnixConnector, Body>,
    pub analytics: Arc<Analytics>,
    pub secret: Secret,
    pub admin_token: String,
    pub auth: Auth,
    pub slots: Arc<Semaphore>,
    pub force_stop: CancellationToken,
    pub tunnels: TaskTracker,
    pub sockjs: Arc<crate::sockjs::Hub>,
    pub control: OnceLock<Weak<crate::reload::Control>>,
}
impl AppState {
    pub fn native_pool(&self, app: crate::config::App) -> Pool {
        let mut pool = Pool::native(
            app,
            &self.config,
            self.startup.clone(),
            self.log_tasks.clone(),
        );
        if let Some(control) = self.control.get().and_then(Weak::upgrade) {
            control.inherit_discovered(&mut pool);
        }
        pool
    }
    pub fn current(self: &Arc<Self>) -> Arc<Self> {
        self.control
            .get()
            .and_then(Weak::upgrade)
            .map_or_else(|| self.clone(), |c| c.snapshot())
    }
    pub fn pools(&self) -> Vec<Arc<Pool>> {
        self.pools.read().unwrap_or_else(|p| p.into_inner()).clone()
    }
}
pub async fn handle(
    State(control): State<Arc<crate::reload::Control>>,
    Extension(listener): Extension<SocketAddr>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request<Body>,
) -> Response {
    let state = control.snapshot();
    if !state.templates_enabled {
        return handle_request(state, listener, peer, req).await;
    }
    let head = req.method() == Method::HEAD;
    let path = crate::config::canonical_path(req.uri().path()).unwrap_or_default();
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let server = state.config.select_server(listener, host);
    let fallback = server
        .and_then(|i| state.config.servers.get(i))
        .and_then(|s| s.template_dir.clone())
        .or_else(|| state.config.template_dir.clone());
    let pools = state.pools();
    let directory = pools
        .iter()
        .filter(|p| Some(p.server()) == server && matches_mount(&path, p.mount()))
        .map(|p| {
            (
                p.mount().len(),
                match p.as_ref() {
                    Pool::Native { app, .. } => app.template_dir.clone(),
                    _ => None,
                },
            )
        })
        .chain(
            state
                .config
                .sites
                .iter()
                .filter(|s| Some(s.app_defaults.server) == server && matches_mount(&path, &s.mount))
                .map(|s| (s.mount.len(), s.app_defaults.template_dir.clone())),
        )
        .max_by_key(|(len, _)| *len)
        .and_then(|(_, dir)| dir)
        .or(fallback);
    let mut response = handle_request(state, listener, peer, req).await;
    if let Some(directory) = directory
        && let Some(problem) = response.extensions().get::<Problem>()
    {
        let data = serde_json::json!({"title":response.status().canonical_reason().unwrap_or("Application unavailable"), "message":problem.0, "detail":"", "detailHTML":"", "console":""});
        match crate::templates::render(
            &directory,
            &format!("error-{}", response.status().as_u16()),
            data,
        )
        .await
        {
            Ok(Some(html)) => {
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                );
                response
                    .headers_mut()
                    .insert(header::CONTENT_LENGTH, html.len().into());
                *response.body_mut() = if head {
                    Body::empty()
                } else {
                    Body::from(html)
                };
            }
            Err(error) => tracing::error!(%error, "Cannot render custom error template"),
            Ok(None) => {}
        }
    }
    response
}
async fn handle_request(
    state: Arc<AppState>,
    listener: SocketAddr,
    peer: SocketAddr,
    mut req: Request<Body>,
) -> Response {
    let start = Instant::now();
    if req.method() == Method::CONNECT {
        return problem(StatusCode::METHOD_NOT_ALLOWED, "CONNECT is not supported");
    }
    let _permit = match state.slots.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Server is busy; retry shortly",
            );
        }
    };
    let path = crate::config::canonical_path(req.uri().path())
        .unwrap_or_else(|| req.uri().path().to_owned());
    if path == "/__shinyharbor__/icon.svg" && matches!(*req.method(), Method::GET | Method::HEAD) {
        let mut response = crate::server::icon().await;
        if req.method() == Method::HEAD {
            *response.body_mut() = Body::empty();
        }
        return response;
    }
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().authority().map(|a| a.as_str()))
        .unwrap_or("");
    let Some(server) = state.config.select_server(listener, host) else {
        return problem(
            StatusCode::NOT_FOUND,
            "No virtual server matches this hostname",
        );
    };
    let pools: Vec<_> = state
        .pools()
        .into_iter()
        .filter(|p| p.server() == server)
        .collect();
    let mut pool = pools
        .iter()
        .filter(|p| matches_mount(&path, p.mount()))
        .max_by_key(|p| p.mount().len())
        .cloned();
    let site = state
        .config
        .sites
        .iter()
        .filter(|s| s.app_defaults.server == server && matches_mount(&path, &s.mount))
        .max_by_key(|s| s.mount.len());
    let redirect = state
        .config
        .redirects
        .iter()
        .filter(|r| r.server == server)
        .filter(|r| {
            if r.exact {
                path == r.mount
            } else {
                matches_mount(&path, &r.mount)
            }
        })
        .max_by_key(|r| r.mount.len());
    let route_length = pool
        .as_ref()
        .map_or(0, |p| p.mount().len())
        .max(site.map_or(0, |s| s.mount.len()));
    if let Some(r) = redirect.filter(|r| r.mount.len() >= route_length) {
        let suffix = if r.exact {
            ""
        } else {
            path.strip_prefix(r.mount.trim_end_matches('/'))
                .unwrap_or("")
        };
        let target = redirect_target(&r.target, suffix, req.uri().query());
        return crate::site::redirect(
            &target,
            StatusCode::from_u16(r.status).unwrap_or(StatusCode::FOUND),
        );
    }
    if let Some(site) = site.filter(|s| {
        pool.as_ref()
            .is_none_or(|p| s.mount.len() > p.mount().len())
    }) {
        let public_tls = !matches!(state.config.listener_tls(listener), Tls::Off {})
            || state.config.public_origin.is_some();
        if (!site.app_defaults.allowed_users.is_empty()
            || !site.app_defaults.allowed_groups.is_empty())
            && !public_tls
            && !peer.ip().is_loopback()
        {
            return problem(
                StatusCode::UPGRADE_REQUIRED,
                "Authentication requires HTTPS",
            );
        }
        match crate::site::resolve(site, &path, &state.private_paths).await {
            Ok(crate::site::Target::App(app)) => {
                if !matches_mount(&path, &app.mount) {
                    return problem(StatusCode::BAD_REQUEST, "Use the canonical application URL");
                }
                let mut all = state.pools.write().unwrap_or_else(|p| p.into_inner());
                pool = if let Some(existing) = all
                    .iter()
                    .find(|p| p.server() == server && p.mount() == app.mount)
                {
                    Some(existing.clone())
                } else {
                    if all.len() >= state.config.apps.len() + state.config.upstreams.len() + 4096 {
                        return problem(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Discovered application limit reached",
                        );
                    }
                    let new = Arc::new(state.native_pool(*app));
                    all.push(new.clone());
                    Some(new)
                };
            }
            Ok(crate::site::Target::Static(file, resolved_site)) => {
                if let Err(status) = state
                    .auth
                    .authorize_request(
                        req.headers(),
                        peer.ip(),
                        &site.app_defaults.allowed_users,
                        &site.app_defaults.allowed_groups,
                        site.app_defaults.auth_ignore_case,
                    )
                    .await
                {
                    return auth_error(status, state.auth.basic_challenge());
                }
                let response =
                    crate::site::serve(&resolved_site, file, req, &state.private_paths).await;
                return response.map(|body| {
                    Body::new(LimitedBody {
                        inner: body,
                        _permit,
                    })
                });
            }
            Err(status) => return problem(status, "The requested site resource is unavailable"),
        }
    }
    let Some(pool) = pool else {
        if path == "/" && req.method() == Method::GET {
            return index(&pools);
        }
        return problem(
            StatusCode::NOT_FOUND,
            "No application is configured at this address",
        );
    };
    let public_tls = !matches!(state.config.listener_tls(listener), Tls::Off {})
        || state.config.public_origin.is_some();
    let (allowed_users, allowed_groups, ignore_case) = match pool.access_policy() {
        Ok(policy) => policy,
        Err(error) => {
            tracing::warn!(%error, "Invalid application access policy");
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Invalid application configuration",
            );
        }
    };
    if (!allowed_users.is_empty() || !allowed_groups.is_empty())
        && !public_tls
        && !peer.ip().is_loopback()
    {
        return problem(
            StatusCode::UPGRADE_REQUIRED,
            "Authentication requires HTTPS",
        );
    }
    let user = match state
        .auth
        .authorize_request(
            req.headers(),
            peer.ip(),
            &allowed_users,
            &allowed_groups,
            ignore_case,
        )
        .await
    {
        Ok(user) => user,
        Err(status) => {
            return auth_error(status, state.auth.basic_challenge());
        }
    };
    if pool.is_native() {
        let relative = if pool.mount() == "/" {
            path.as_str()
        } else {
            path.strip_prefix(pool.mount()).unwrap_or("")
        };
        if relative.starts_with("/__assets__/")
            || relative.starts_with("/__sockjs__/")
            || relative == "/__sockjs__"
        {
            return crate::sockjs::handle(state, pool, user, peer, req, _permit).await;
        }
        if security::is_websocket(req.headers())
            && matches!(pool.as_ref(), Pool::Native { app, .. } if !crate::sockjs::enabled(&app.disable_protocols, "websocket"))
        {
            return problem(StatusCode::NOT_FOUND, "WebSocket transport is disabled");
        }
    }
    if pool.mount() != "/" && path == pool.mount() {
        let query = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let mut r = StatusCode::PERMANENT_REDIRECT.into_response();
        let Ok(location) = HeaderValue::from_str(&format!("{path}/{query}")) else {
            return problem(StatusCode::BAD_REQUEST, "Invalid redirect URL");
        };
        r.headers_mut().insert(header::LOCATION, location);
        return r;
    }
    let websocket = security::is_websocket(req.headers());
    if req.headers().contains_key(header::UPGRADE) && !websocket {
        return problem(
            StatusCode::BAD_REQUEST,
            "Only WebSocket upgrades are supported",
        );
    }
    let is_document_request = req.method() == Method::GET
        && (req
            .headers()
            .get("sec-fetch-dest")
            .is_some_and(|v| v == "document" || v == "iframe")
            || req
                .headers()
                .get(header::ACCEPT)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|s| s.contains("text/html")));
    let new_document =
        is_document_request && (path == format!("{}/", pool.mount().trim_end_matches('/')));
    let cookie_name = format!("harbor_{}", hex::encode(pool.name().as_bytes()));
    let affinity = pool.session_affinity(&path).or_else(|| {
        security::cookie(req.headers(), &cookie_name)
            .and_then(|t| state.secret.verify(pool.name(), &t, u64::MAX))
    });
    let ip = security::client_ip(peer.ip(), req.headers(), &state.config.trusted_proxies);
    let mut event = state.analytics.event(ip, req.headers(), pool.name());
    let lease = match pool
        .select(affinity.as_deref(), websocket, new_document)
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(app = %pool.name(), error = %e, "Worker acquisition failed");
            record_error(&state, event, start);
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "The application is not ready or this session has ended. Reload the page; the server log contains details.",
            );
        }
    };
    let backend_id = lease.worker.id.clone();
    let outgoing_uri = match backend_uri(
        req.uri(),
        pool.mount(),
        &lease.worker.origin,
        pool.is_native(),
    ) {
        Ok(uri) => uri,
        Err(_) => return problem(StatusCode::BAD_REQUEST, "Invalid request URI"),
    };
    #[cfg(unix)]
    let outgoing_uri = match &lease.worker.socket {
        Some(socket) => hyperlocal::Uri::new(
            socket,
            outgoing_uri.path_and_query().map_or("/", |p| p.as_str()),
        )
        .into(),
        None => outgoing_uri,
    };
    // Register the downstream upgrade before moving the request into the client.
    let downstream_upgrade = websocket.then(|| hyper::upgrade::on(&mut req));
    let host = req
        .headers()
        .get(header::HOST)
        .cloned()
        .or_else(|| req.uri().authority().and_then(|a| a.as_str().parse().ok()));
    let Some(host) = host else {
        return problem(StatusCode::BAD_REQUEST, "Host header is required");
    };
    security::strip_hop_headers(req.headers_mut(), websocket);
    state.auth.strip_identity_headers(req.headers_mut());
    if pool.is_native() {
        req.headers_mut().remove(header::AUTHORIZATION);
    }
    for key in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-forwarded-host",
        "x-forwarded-port",
        "x-real-ip",
        "shiny-server-credentials",
        "shiny-shared-secret",
        "x-forwarded-user",
        "x-auth-request-user",
        "x-harbor-user",
    ] {
        req.headers_mut().remove(key);
    }
    req.headers_mut().insert(
        "x-forwarded-for",
        ip.to_string()
            .parse()
            .expect("An IP address is a valid header"),
    );
    req.headers_mut().insert(
        "x-real-ip",
        ip.to_string()
            .parse()
            .expect("An IP address is a valid header"),
    );
    req.headers_mut().insert("x-forwarded-host", host.clone());
    req.headers_mut().insert(header::HOST, host);
    req.headers_mut().insert(
        "x-forwarded-proto",
        HeaderValue::from_static(if public_tls { "https" } else { "http" }),
    );
    if let Some(user) = user {
        if let Ok(v) = HeaderValue::from_str(&user.user) {
            req.headers_mut().insert("x-harbor-user", v);
        }
        if pool.is_native() {
            req.headers_mut().remove(header::AUTHORIZATION);
            let credentials = serde_json::to_string(&user).expect("Identity is serializable");
            if let Ok(value) = HeaderValue::from_str(&credentials) {
                req.headers_mut().insert("shiny-server-credentials", value);
            }
        }
    }
    if let Some(secret) = &lease.worker.shared_secret {
        req.headers_mut().insert(
            "shiny-shared-secret",
            HeaderValue::from_str(secret).expect("Generated secret is ASCII"),
        );
    }
    *req.uri_mut() = outgoing_uri;
    // Browser HTTP/2 becomes HTTP/1 to R. Hop-by-hop framing is handled by hyper.
    *req.version_mut() = http::Version::HTTP_11;
    let bytes_in = Arc::new(AtomicU64::new(0));
    let req = req.map(|body| {
        Body::new(CountBody {
            inner: body,
            count: bytes_in.clone(),
        })
    });
    let result = tokio::time::timeout(
        Duration::from_secs(state.config.response_header_timeout_seconds),
        async {
            #[cfg(unix)]
            if lease.worker.socket.is_some() {
                return state.socket_client.request(req).await;
            }
            state.client.request(req).await
        },
    )
    .await;
    let mut upstream = match result {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            tracing::warn!(app = %pool.name(), error = %e, "Upstream request failed; not replayed");
            record_error(&state, event, start);
            return problem(
                StatusCode::BAD_GATEWAY,
                "The application connection failed. Reload the application.",
            );
        }
        Err(_) => {
            record_error(&state, event, start);
            return problem(
                StatusCode::GATEWAY_TIMEOUT,
                "The application did not send response headers before the configured deadline",
            );
        }
    };
    let status = upstream.status();
    let upgraded = websocket && status == StatusCode::SWITCHING_PROTOCOLS;
    let upgrade = upgraded.then(|| hyper::upgrade::on(&mut upstream));
    security::strip_hop_headers(upstream.headers_mut(), upgraded);
    if let Pool::Native { app, .. } = pool.as_ref() {
        use crate::config::FrameOptions;
        match crate::legacy::app_settings(app)
            .map_or_else(|_| app.frame_options.clone(), |a| a.frame_options)
        {
            FrameOptions::Allow => {
                upstream.headers_mut().remove(header::X_FRAME_OPTIONS);
            }
            FrameOptions::Deny => {
                upstream
                    .headers_mut()
                    .insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
            }
            FrameOptions::Sameorigin => {
                upstream.headers_mut().insert(
                    header::X_FRAME_OPTIONS,
                    HeaderValue::from_static("SAMEORIGIN"),
                );
            }
            FrameOptions::AllowFrom(origin) => {
                if let Ok(value) = HeaderValue::from_str(&format!("ALLOW-FROM {origin}")) {
                    upstream
                        .headers_mut()
                        .insert(header::X_FRAME_OPTIONS, value);
                }
            }
        }
    }
    // Never collapse repeated Set-Cookie headers or rewrite response bodies.
    if affinity.as_deref() != Some(backend_id.as_str()) {
        let token = state.secret.sign(pool.name(), &backend_id);
        let value = format!(
            "{cookie_name}={token}; Path={}; HttpOnly; SameSite=Lax{}",
            pool.mount(),
            if public_tls { "; Secure" } else { "" }
        );
        if let Ok(value) = HeaderValue::from_str(&value) {
            upstream.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    if let Some(e) = event.as_mut() {
        e.request = 1;
        e.header_ms = start.elapsed().as_millis() as u64;
        e.error = u64::from(status.is_server_error());
        e.websocket = u64::from(upgraded);
        e.page = u64::from(
            is_document_request
                && status.is_success()
                && upstream
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|s| s.starts_with("text/html")),
        );
    }
    if let (Some(from_client), Some(from_server)) = (downstream_upgrade, upgrade) {
        state.analytics.record(event.clone());
        if let Some(e) = event.as_mut() {
            e.request = 0;
            e.page = 0;
            e.websocket = 0;
            e.error = 0;
            e.header_ms = 0;
        }
        let state2 = state.clone();
        let session_timeout = match pool.session_timeout() {
            Ok(timeout) => timeout,
            Err(_) => {
                return problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Invalid application configuration",
                );
            }
        };
        state.tunnels.spawn(async move {
            let _lease = lease;
            let _permit = _permit;
            let connected = tokio::time::timeout(Duration::from_secs(15), async {
                tokio::try_join!(from_client, from_server)
            })
            .await;
            let started = Instant::now();
            let outcome = match connected {
                Ok(Ok((a, b))) => {
                    let mut a = TokioIo::new(a);
                    let mut b = TokioIo::new(b);
                    tokio::select! {
                        copy = crate::activity::copy(&mut a, &mut b, session_timeout) => copy.ok(),
                        _ = state2.force_stop.cancelled() => None,
                    }
                }
                _ => None,
            };
            if let Some(e) = event.as_mut() {
                e.websocket_ms = started.elapsed().as_millis() as u64;
                if let Some((up, down)) = outcome {
                    e.bytes_in = up;
                    e.bytes_out = down;
                } else {
                    e.interrupted = 1;
                }
            }
            state2.analytics.record(event);
        });
        return upstream.map(|_| Body::empty());
    }
    let (parts, body) = upstream.into_parts();
    Response::from_parts(
        parts,
        Body::new(ObservedBody {
            inner: Body::new(body),
            event,
            analytics: state.analytics.clone(),
            bytes_in,
            _lease: lease,
            _permit,
            complete: false,
        }),
    )
}
fn redirect_target(target: &str, suffix: &str, query: Option<&str>) -> String {
    let (target, fragment) = target
        .split_once('#')
        .map_or((target, None), |(a, b)| (a, Some(b)));
    let (base, configured_query) = target
        .split_once('?')
        .map_or((target, None), |(a, b)| (a, Some(b)));
    let mut result = if suffix.is_empty() {
        base.to_owned()
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    };
    let queries: Vec<_> = configured_query
        .into_iter()
        .chain(query)
        .filter(|q| !q.is_empty())
        .collect();
    if !queries.is_empty() {
        result.push('?');
        result.push_str(&queries.join("&"));
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    result
}
fn record_error(state: &AppState, mut event: Option<Event>, started: Instant) {
    if let Some(e) = event.as_mut() {
        e.request = 1;
        e.error = 1;
        e.header_ms = started.elapsed().as_millis() as u64;
    }
    state.analytics.record(event);
}
fn auth_error(status: StatusCode, basic: bool) -> Response {
    let mut response = problem(status, "Authentication is required or access was denied");
    if status == StatusCode::UNAUTHORIZED && basic {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"ShinyHarbor\", charset=\"UTF-8\""),
        );
    }
    response
}
pub fn backend_uri(
    uri: &Uri,
    mount: &str,
    origin: &str,
    strip_mount: bool,
) -> Result<Uri, http::uri::InvalidUri> {
    let path = if strip_mount && mount != "/" {
        crate::config::relative_path(uri.path(), mount)
            .filter(|s| !s.is_empty())
            .unwrap_or("/")
    } else {
        uri.path()
    };
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    format!("{origin}{path}{query}").parse()
}
struct CountBody {
    inner: Body,
    count: Arc<AtomicU64>,
}
struct LimitedBody {
    inner: Body,
    _permit: OwnedSemaphorePermit,
}
impl HttpBody for LimitedBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
impl HttpBody for CountBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    self.count.fetch_add(data.len() as u64, Ordering::Relaxed);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => other,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
struct ObservedBody {
    inner: Body,
    event: Option<Event>,
    analytics: Arc<Analytics>,
    bytes_in: Arc<AtomicU64>,
    _lease: Lease,
    _permit: OwnedSemaphorePermit,
    complete: bool,
}
impl HttpBody for ObservedBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref()
                    && let Some(e) = self.event.as_mut()
                {
                    e.bytes_out += data.len() as u64;
                }
                if self.inner.is_end_stream() {
                    self.complete = true;
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(None) => {
                self.complete = true;
                Poll::Ready(None)
            }
            other => other,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
impl Drop for ObservedBody {
    fn drop(&mut self) {
        if let Some(e) = self.event.as_mut() {
            e.bytes_in = self.bytes_in.load(Ordering::Relaxed);
            e.interrupted = u64::from(!self.complete && !self.inner.is_end_stream());
        }
        self.analytics.record(self.event.take());
    }
}
pub fn problem(code: StatusCode, text: &str) -> Response {
    let mut response = (code, text.to_owned()).into_response();
    response.extensions_mut().insert(Problem(text.to_owned()));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if code == StatusCode::SERVICE_UNAVAILABLE {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("3"));
    }
    response
}
#[derive(Clone)]
struct Problem(String);
fn index(pools: &[Arc<Pool>]) -> Response {
    let links: String = pools.iter().map(|p| {
        let title = p.name().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let href = p.mount().trim_end_matches('/').replace('&', "&amp;").replace('"', "&quot;");
        format!("<a class=\"app-card\" href=\"{href}/\"><span class=\"tile\" aria-hidden=\"true\">&#9671;</span><h3>{title}</h3><p>An interactive application hosted here.</p><span class=\"go\">Open application &#8599;</span></a>")
    }).collect();
    axum::response::Html(crate::site::catalog_html(&links, "", pools.len())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirects_preserve_query_and_fragment_without_creating_network_paths() {
        assert_eq!(
            redirect_target("/new?fixed=1#section", "/a%20b", Some("x=1&x=2")),
            "/new/a%20b?fixed=1&x=1&x=2#section"
        );
        assert_eq!(
            redirect_target("/", "//evil.example/x", None),
            "/evil.example/x"
        );
        assert_eq!(
            redirect_target("https://example.org/base/", "/child", None),
            "https://example.org/base/child"
        );
    }
    #[test]
    fn query_and_encoding_are_preserved() {
        let u: Uri = "/app/session/a/download/a%20b?x=1&x=2&v=%2F"
            .parse()
            .unwrap();
        assert_eq!(
            backend_uri(&u, "/app", "http://127.0.0.1:9000", true)
                .unwrap()
                .to_string(),
            "http://127.0.0.1:9000/session/a/download/a%20b?x=1&x=2&v=%2F"
        );
        assert_eq!(
            backend_uri(&u, "/app", "http://127.0.0.1:9000", false)
                .unwrap()
                .path(),
            u.path()
        );
    }
}
