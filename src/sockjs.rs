//! SockJS framing and HTTP transports, bridged to the installed Shiny runtime.
//! Each session owns exactly one R connection, including between polling calls.
use crate::{
    auth::Identity,
    pool::Pool,
    proxy::{AppState, problem},
    security,
};
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{
        FromRequestParts, WebSocketUpgrade,
        ws::{Message as ClientMessage, WebSocket},
    },
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{
    Mutex as AsyncMutex, OnceCell, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, mpsc,
};
use tokio_tungstenite::tungstenite::{
    Message, client::IntoClientRequest, protocol::WebSocketConfig,
};
use tokio_util::sync::CancellationToken;

mod reconnect;
const CACHE: &str = "no-store, no-cache, no-transform, must-revalidate, max-age=0";
const CLOSED: &str = "c[3000,\"Session closed\"]";
const BUSY: &str = "c[2010,\"Another connection still open\"]";
const PROTOCOLS: &[&str] = &[
    "websocket",
    "xhr-streaming",
    "xdr-streaming",
    "eventsource",
    "iframe-eventsource",
    "htmlfile",
    "iframe-htmlfile",
    "xhr-polling",
    "xdr-polling",
    "iframe-xhr-polling",
    "jsonp-polling",
    "streaming",
    "polling",
];

pub fn valid_protocol(name: &str) -> bool {
    PROTOCOLS.contains(&name)
}
pub fn enabled(disabled: &[String], name: &str) -> bool {
    !disabled.iter().any(|p| {
        p == name
            || (p == "streaming"
                && matches!(
                    name,
                    "xhr-streaming"
                        | "xdr-streaming"
                        | "eventsource"
                        | "iframe-eventsource"
                        | "htmlfile"
                        | "iframe-htmlfile"
                ))
            || (p == "polling" && name.ends_with("polling"))
            || (p == "iframe-eventsource" && name == "eventsource")
            || (p == "iframe-htmlfile" && name == "htmlfile")
    })
}

type Key = (String, String);
struct Entry {
    session: OnceCell<Arc<Session>>,
    created: Instant,
}
struct Queued {
    text: String,
    _budget: OwnedSemaphorePermit,
}
struct Batch {
    messages: Vec<String>,
    _budget: OwnedSemaphorePermit,
}
struct Session {
    owner: Option<Identity>,
    incoming: Arc<AsyncMutex<mpsc::Receiver<Queued>>>,
    outgoing: mpsc::Sender<Batch>,
    budget: Arc<Semaphore>,
    attached: AtomicBool,
    detached: Mutex<Instant>,
    stop: CancellationToken,
    done: AtomicBool,
}
pub struct Hub {
    entries: Mutex<HashMap<Key, Arc<Entry>>>,
    budget: Arc<Semaphore>,
    reconnect: reconnect::Registry,
}
impl Hub {
    pub fn new(memory_budget: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            budget: Arc::new(Semaphore::new(memory_budget)),
            reconnect: reconnect::Registry::default(),
        }
    }
}
impl Hub {
    pub fn reap(&self, delay: Duration) {
        self.reconnect.reap();
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, entry| {
                if let Some(session) = entry.session.get() {
                    if !session.attached.load(Ordering::Acquire)
                        && session
                            .detached
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .elapsed()
                            > delay
                    {
                        session.stop.cancel();
                        return false;
                    }
                } else if entry.created.elapsed() > Duration::from_secs(300) {
                    return false;
                }
                true
            });
    }
    fn entry(&self, key: Key, create: bool, limit: usize) -> Result<Arc<Entry>, StatusCode> {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = entries.get(&key) {
            return Ok(entry.clone());
        }
        if !create {
            return Err(StatusCode::NOT_FOUND);
        }
        if entries.len() >= limit.min(4096) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let entry = Arc::new(Entry {
            session: OnceCell::new(),
            created: Instant::now(),
        });
        entries.insert(key, entry.clone());
        Ok(entry)
    }
}
struct Attachment {
    session: Arc<Session>,
    receiver: OwnedMutexGuard<mpsc::Receiver<Queued>>,
}
impl Attachment {
    fn new(session: Arc<Session>) -> Option<Self> {
        let receiver = session.incoming.clone().try_lock_owned().ok()?;
        session.attached.store(true, Ordering::Release);
        Some(Self { session, receiver })
    }
    async fn next(&mut self, heartbeat: Duration) -> String {
        tokio::select! {
            frame = self.receiver.recv() => frame.map_or_else(|| CLOSED.to_owned(), |f| f.text),
            _ = self.session.stop.cancelled() => CLOSED.to_owned(),
            _ = tokio::time::sleep(heartbeat) => "h".to_owned(),
        }
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        *self
            .session
            .detached
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Instant::now();
        self.session.attached.store(false, Ordering::Release);
    }
}

pub async fn handle(
    state: Arc<AppState>,
    pool: Arc<Pool>,
    user: Option<Identity>,
    peer: SocketAddr,
    req: Request<Body>,
    permit: OwnedSemaphorePermit,
) -> Response {
    let Pool::Native { app, .. } = pool.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let app = match crate::legacy::app_settings(app) {
        Ok(app) => app,
        Err(error) => {
            tracing::warn!(%error, "Invalid application override");
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Invalid application configuration",
            );
        }
    };
    let path = crate::config::relative_path(req.uri().path(), pool.mount())
        .unwrap_or("")
        .to_owned();
    if path.starts_with("/__assets__/") {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return method("GET, HEAD");
        }
        let source = match path.as_str() {
            "/__assets__/sockjs.min.js" => include_str!("../assets/vendor/sockjs-1.6.1.min.js"),
            "/__assets__/shinyharbor-client.js" => include_str!("../assets/shinyharbor-client.js"),
            _ => return StatusCode::NOT_FOUND.into_response(),
        };
        let mut response = text(source.to_owned(), "application/javascript; charset=UTF-8");
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600"),
        );
        if req.method() == Method::HEAD {
            *response.body_mut() = Body::empty();
        }
        return response;
    }
    let path = path.strip_prefix("/__sockjs__").unwrap_or("");
    let (worker, path) = if let Some(rest) = path.strip_prefix("/w=") {
        let (ticket, _) = rest.split_once('/').unwrap_or((rest, ""));
        let Some(worker) = state.secret.verify(pool.name(), ticket, u64::MAX) else {
            return problem(StatusCode::FORBIDDEN, "Invalid worker ticket");
        };
        (Some(worker), &rest[ticket.len()..])
    } else {
        (None, path)
    };
    let (robust, path) = if path.starts_with("/n=") || path.starts_with("/o=") {
        let (id, _) = path[3..].split_once('/').unwrap_or((&path[3..], ""));
        if !identifier(id) {
            return StatusCode::NOT_FOUND.into_response();
        }
        (
            Some((id.to_owned(), path.starts_with("/o="))),
            &path[3 + id.len()..],
        )
    } else {
        (None, path)
    };
    // Cross-origin authenticated transports would expose application sessions.
    if !same_origin(req.headers(), state.config.public_origin.as_deref()) {
        return problem(
            StatusCode::FORBIDDEN,
            "SockJS Origin does not match this server",
        );
    }
    let headers = req.headers().clone();
    if req.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("OPTIONS, GET, POST"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("31536000"),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000"),
        );
        return cors(response, &headers);
    }
    if matches!(path, "" | "/") {
        return if req.method() == Method::GET {
            text("Welcome to SockJS!\n".into(), "text/plain; charset=UTF-8")
        } else {
            method("GET")
        };
    }
    if path == "/info" {
        if req.method() != Method::GET {
            return method("GET, OPTIONS");
        }
        let value = serde_json::json!({"websocket": enabled(&app.disable_protocols, "websocket"), "cookie_needed": true,
            "origins": ["*:*"], "entropy": rand::random::<u32>()});
        return cors(
            text(value.to_string(), "application/json; charset=UTF-8"),
            &headers,
        );
    }
    if path.starts_with("/iframe") && path.ends_with(".html") && !path[1..].contains('/') {
        if req.method() != Method::GET {
            return method("GET");
        }
        return text(
            format!(
                "<!DOCTYPE html><html><head><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\"/><meta charset=\"UTF-8\"/><script src=\"{}/__assets__/sockjs.min.js\"></script><script>document.domain=document.domain;SockJS.bootstrap_iframe();</script></head><body><h2>Don't panic!</h2><p>This is a SockJS hidden iframe.</p></body></html>",
                pool.mount().trim_end_matches('/')
            ),
            "text/html; charset=UTF-8",
        );
    }
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if parts.len() != 3 || !identifier(parts[0]) || !identifier(parts[1]) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let transport = parts[2];
    let protocol = match transport {
        "websocket" => "websocket",
        "xhr" | "xhr_send" => "xhr-polling",
        "xhr_streaming" => "xhr-streaming",
        "eventsource" => "eventsource",
        "htmlfile" => "htmlfile",
        "jsonp" | "jsonp_send" => "jsonp-polling",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let aliases: &[&str] = match transport {
        "xhr" => &["xhr-polling", "xdr-polling", "iframe-xhr-polling"],
        "xhr_streaming" => &["xhr-streaming", "xdr-streaming"],
        "eventsource" => &["eventsource", "iframe-eventsource"],
        "htmlfile" => &["htmlfile", "iframe-htmlfile"],
        _ => &[],
    };
    if !transport.ends_with("_send")
        && !enabled(&app.disable_protocols, protocol)
        && !aliases.iter().any(|p| enabled(&app.disable_protocols, p))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let post = matches!(
        transport,
        "xhr" | "xhr_streaming" | "xhr_send" | "jsonp_send"
    );
    if req.method() != if post { Method::POST } else { Method::GET } {
        return method(if post {
            "POST, OPTIONS"
        } else {
            "GET, OPTIONS"
        });
    }
    let callback = if matches!(transport, "htmlfile" | "jsonp") {
        match url::form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
            .find(|(k, _)| k == "c")
            .map(|(_, v)| v.into_owned())
        {
            Some(c) if callback_name(&c) => c,
            _ => {
                return problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "A valid callback parameter is required",
                );
            }
        }
    } else {
        String::new()
    };
    let key = (
        pool.name().to_owned(),
        match &robust {
            Some((id, _)) => format!("{id}:{}", parts[1]),
            None => parts[1].to_owned(),
        },
    );
    let entry = match state.sockjs.entry(
        key,
        !transport.ends_with("_send"),
        state.config.max_inflight,
    ) {
        Ok(e) => e,
        Err(status) => return status.into_response(),
    };
    let session = match entry
        .session
        .get_or_try_init(|| async {
            if let Some((id, existing)) = &robust {
                reconnect::create(
                    &state,
                    &pool,
                    user.clone(),
                    peer,
                    &headers,
                    worker.as_deref(),
                    id,
                    *existing,
                    app.reconnect,
                )
                .await
            } else {
                create_session(
                    state.clone(),
                    pool.clone(),
                    user.clone(),
                    peer,
                    &headers,
                    worker.as_deref(),
                )
                .await
            }
        })
        .await
    {
        Ok(s) if s.owner == user => s.clone(),
        Ok(_) => return StatusCode::FORBIDDEN.into_response(),
        Err(error) => {
            tracing::warn!(%error, "SockJS session startup failed");
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "Application connection unavailable",
            );
        }
    };
    if transport.ends_with("_send") {
        if session.done.load(Ordering::Acquire) || session.stop.is_cancelled() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let form = headers.get(header::CONTENT_TYPE).is_some_and(|h| {
            h.as_bytes()
                .starts_with(b"application/x-www-form-urlencoded")
        });
        let body = match to_bytes(req.into_body(), state.config.sockjs.max_message_bytes).await {
            Ok(b) => b,
            Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        };
        let payload = if form {
            url::form_urlencoded::parse(&body)
                .find(|(k, _)| k == "d")
                .map(|(_, v)| v.into_owned())
        } else {
            String::from_utf8(body.to_vec()).ok()
        };
        let Some(payload) = payload.filter(|p| !p.is_empty()) else {
            return problem(StatusCode::INTERNAL_SERVER_ERROR, "Payload expected.");
        };
        let messages = match decode(&payload) {
            Ok(m) => m,
            Err(_) => return problem(StatusCode::INTERNAL_SERVER_ERROR, "Broken JSON encoding."),
        };
        if send(&session, messages).is_err() {
            session.stop.cancel();
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let response = if transport == "jsonp_send" {
            text("ok".into(), "text/plain; charset=UTF-8")
        } else {
            StatusCode::NO_CONTENT.into_response()
        };
        return cors(response, &headers);
    }
    if transport == "websocket" {
        let (mut parts, _) = req.into_parts();
        let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(u) => u,
            Err(e) => return e.into_response(),
        };
        let heartbeat = Duration::from_secs_f64(state.config.sockjs_heartbeat_seconds);
        return upgrade
            .max_message_size(state.config.sockjs.max_message_bytes)
            .max_frame_size(state.config.sockjs.max_message_bytes)
            .on_upgrade(move |socket| async move {
                let _permit = permit;
                websocket(socket, session, heartbeat).await;
            })
            .into_response();
    }
    let Some(attachment) = Attachment::new(session) else {
        return cors(
            text(frame(BUSY, transport, &callback), mime(transport)),
            &headers,
        );
    };
    let heartbeat = Duration::from_secs_f64(state.config.sockjs_heartbeat_seconds);
    let transport = transport.to_owned();
    let prefix = match transport.as_str() {
        "xhr_streaming" => format!("{}\n", "h".repeat(2048)),
        "eventsource" => "\r\n".into(),
        "htmlfile" => format!(
            "<!doctype html><html><head><meta charset=\"UTF-8\"></head><body><script>document.domain=document.domain;var c=parent.{callback};c.start();function p(d){{c.message(d);}}window.onload=function(){{c.stop();}};</script>{}\r\n",
            " ".repeat(1024)
        ),
        _ => String::new(),
    };
    let content_type = mime(&transport);
    let stream = futures_util::stream::unfold(
        (attachment, Some(prefix), false, 0usize, permit),
        move |(mut attached, prefix, done, bytes, permit)| {
            let transport = transport.clone();
            let callback = callback.clone();
            async move {
                if done {
                    return None;
                }
                if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
                    return Some((
                        Ok::<_, Infallible>(Bytes::from(prefix)),
                        (attached, None, false, bytes, permit),
                    ));
                }
                let value = attached.next(heartbeat).await;
                let body = frame(&value, &transport, &callback);
                let bytes = bytes + body.len();
                let done = value.starts_with('c')
                    || matches!(transport.as_str(), "xhr" | "jsonp")
                    || bytes >= 128 * 1024;
                Some((Ok(Bytes::from(body)), (attached, None, done, bytes, permit)))
            }
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    cors(response, &headers)
}

async fn create_session(
    state: Arc<AppState>,
    pool: Arc<Pool>,
    user: Option<Identity>,
    peer: SocketAddr,
    headers: &HeaderMap,
    worker: Option<&str>,
) -> anyhow::Result<Arc<Session>> {
    let (lease, mut backend) =
        connect_backend(&state, &pool, user.as_ref(), peer, headers, worker).await?;
    let ip = security::client_ip(peer.ip(), headers, &state.config.trusted_proxies);
    let (session, tx, mut input) = session_channels(user, &state.sockjs.budget)?;
    let idle = pool.session_timeout()?;
    let session2 = session.clone();
    let state2 = state.clone();
    let mut event = state.analytics.event(ip, headers, pool.name());
    if let Some(e) = event.as_mut() {
        e.request = 1;
        e.websocket = 1;
    }
    state.analytics.record(event.clone());
    if let Some(e) = event.as_mut() {
        e.request = 0;
        e.websocket = 0;
    }
    state.tunnels.spawn(async move {
        let _lease = lease;
        let mut binding = None;
        let started = Instant::now();
        let mut last_data = tokio::time::Instant::now();
        loop {
            tokio::select! {
                _ = idle_elapsed(last_data, idle) => break,
                _ = state2.force_stop.cancelled() => break,
                _ = session2.stop.cancelled() => break,
                batch = input.recv() => match batch {
                    Some(batch) => {
                        last_data = tokio::time::Instant::now();
                        let mut failed = false;
                        for message in batch.messages {
                            if let Some(e) = event.as_mut() { e.bytes_in += message.len() as u64; }
                            if !tokio::time::timeout(Duration::from_secs(30), backend.send(Message::Text(message.into()))).await.is_ok_and(|r| r.is_ok()) { failed = true; break; }
                        }
                        if failed { break; }
                    }
                    None => break,
                },
                message = backend.next() => match message {
                    Some(Ok(Message::Text(value))) => {
                        last_data = tokio::time::Instant::now();
                        if binding.is_none()
                            && let Ok(message) = serde_json::from_str::<serde_json::Value>(&value)
                            && let Some(token) = message.get("config").and_then(|v| v.get("sessionId")).and_then(|v| v.as_str()).filter(|s| identifier(s)) {
                            binding = Some(_lease.worker.bind_session(token));
                        }
                        if let Some(e) = event.as_mut() { e.bytes_out += value.len() as u64; }
                        let frame = data_frame(&value);
                        if queue(&tx, frame, &state2.sockjs.budget).is_err() { break; }
                    }
                    Some(Ok(Message::Ping(_))) => { if backend.flush().await.is_err() { break; } }
                    Some(Ok(Message::Pong(_))) => {}
                    _ => break,
                }
            }
        }
        session2.done.store(true, Ordering::Release);
        let _ = queue(&tx, CLOSED.to_owned(), &state2.sockjs.budget);
        let _ = tokio::time::timeout(Duration::from_secs(1), backend.close(None)).await;
        if let Some(e) = event.as_mut() { e.websocket_ms = started.elapsed().as_millis() as u64; }
        state2.analytics.record(event);
    });
    Ok(session)
}
async fn idle_elapsed(last_data: tokio::time::Instant, timeout: Option<Duration>) {
    match timeout {
        Some(timeout) => tokio::time::sleep_until(last_data + timeout).await,
        None => std::future::pending().await,
    }
}
type Backend = tokio_tungstenite::WebSocketStream<crate::backend::Stream>;
async fn connect_backend(
    state: &AppState,
    pool: &Pool,
    user: Option<&Identity>,
    peer: SocketAddr,
    headers: &HeaderMap,
    worker: Option<&str>,
) -> anyhow::Result<(crate::pool::Lease, Backend)> {
    let cookie = format!("harbor_{}", hex::encode(pool.name().as_bytes()));
    let affinity = worker.map(str::to_owned).or_else(|| {
        security::cookie(headers, &cookie)
            .and_then(|t| state.secret.verify(pool.name(), &t, u64::MAX))
    });
    let lease = pool.select(affinity.as_deref(), true, false).await?;
    let url = format!(
        "{}/websocket/",
        lease.worker.origin.replacen("http://", "ws://", 1)
    );
    let mut request = url.into_client_request()?;
    let ip = security::client_ip(peer.ip(), headers, &state.config.trusted_proxies);
    let mut original = headers.clone();
    security::strip_hop_headers(&mut original, false);
    security::copy_whitelisted_headers(
        &original,
        request.headers_mut(),
        &pool.whitelist_headers()?,
    );
    state.auth.strip_identity_headers(request.headers_mut());
    for name in [
        header::HOST,
        header::ORIGIN,
        header::USER_AGENT,
        header::COOKIE,
    ] {
        if let Some(value) = headers.get(&name) {
            request.headers_mut().insert(name, value.clone());
        }
    }
    request
        .headers_mut()
        .insert("x-forwarded-for", ip.to_string().parse()?);
    request
        .headers_mut()
        .insert("x-real-ip", ip.to_string().parse()?);
    if let Some(secret) = &lease.worker.shared_secret {
        request
            .headers_mut()
            .insert("shiny-shared-secret", secret.parse()?);
    }
    if let Some(user) = user {
        request.headers_mut().insert(
            "shiny-server-credentials",
            serde_json::to_string(user)?.parse()?,
        );
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(state.config.sockjs.max_message_bytes))
        .max_frame_size(Some(state.config.sockjs.max_message_bytes));
    let (backend, _) = tokio::time::timeout(Duration::from_secs(30), async {
        let stream = crate::backend::Stream::connect(&lease.worker).await?;
        Ok::<_, anyhow::Error>(
            tokio_tungstenite::client_async_with_config(request, stream, Some(config)).await?,
        )
    })
    .await??;
    Ok((lease, backend))
}
fn data_frame(message: &str) -> String {
    format!(
        "a{}",
        serde_json::to_string(&[message]).expect("Text is serializable")
    )
}
fn session_channels(
    user: Option<Identity>,
    budget: &Arc<Semaphore>,
) -> anyhow::Result<(Arc<Session>, mpsc::Sender<Queued>, mpsc::Receiver<Batch>)> {
    let (tx, rx) = mpsc::channel(32);
    let (outgoing, input) = mpsc::channel::<Batch>(8);
    queue(&tx, "o".to_owned(), budget)?;
    let session = Arc::new(Session {
        owner: user,
        incoming: Arc::new(AsyncMutex::new(rx)),
        outgoing,
        budget: budget.clone(),
        attached: AtomicBool::new(false),
        detached: Mutex::new(Instant::now()),
        stop: CancellationToken::new(),
        done: AtomicBool::new(false),
    });
    Ok((session, tx, input))
}

fn queue(tx: &mpsc::Sender<Queued>, text: String, budget: &Arc<Semaphore>) -> anyhow::Result<()> {
    let permit = budget
        .clone()
        .try_acquire_many_owned(u32::try_from(text.len())?)?;
    tx.try_send(Queued {
        text,
        _budget: permit,
    })
    .map_err(|_| anyhow::anyhow!("SockJS receiver is too slow"))
}
async fn websocket(mut socket: WebSocket, session: Arc<Session>, heartbeat: Duration) {
    let Some(mut attachment) = Attachment::new(session.clone()) else {
        let _ = socket.send(ClientMessage::Text(BUSY.into())).await;
        return;
    };
    loop {
        tokio::select! {
            frame = attachment.next(heartbeat) => {
                let closed = frame.starts_with('c');
                if socket.send(ClientMessage::Text(frame.into())).await.is_err() || closed { break; }
            }
            message = socket.recv() => match message {
                Some(Ok(ClientMessage::Text(text))) if text.is_empty() => {},
                Some(Ok(ClientMessage::Text(text))) => match decode(&text) {
                    Ok(messages) => if send(&session, messages).is_err() { break; },
                    Err(_) => break,
                },
                Some(Ok(ClientMessage::Ping(_))) | Some(Ok(ClientMessage::Pong(_))) => {},
                _ => break,
            }
        }
    }
    session.stop.cancel();
    let _ = socket.close().await;
}
fn decode(payload: &str) -> Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(payload)
}
fn send(session: &Session, messages: Vec<String>) -> anyhow::Result<()> {
    let bytes = messages
        .iter()
        .map(|m| m.len() + std::mem::size_of::<String>())
        .sum::<usize>();
    let budget = session
        .budget
        .clone()
        .try_acquire_many_owned(u32::try_from(bytes)?)?;
    session
        .outgoing
        .try_send(Batch {
            messages,
            _budget: budget,
        })
        .map_err(|_| anyhow::anyhow!("SockJS sender is too fast"))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn callback_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_alphabetic()
                        || b == b'_'
                        || b == b'$'
                        || (i > 0 && b.is_ascii_digit())
                })
        })
}
fn mime(transport: &str) -> &'static str {
    match transport {
        "eventsource" => "text/event-stream; charset=UTF-8",
        "htmlfile" => "text/html; charset=UTF-8",
        "jsonp" => "application/javascript; charset=UTF-8",
        _ => "application/javascript; charset=UTF-8",
    }
}
fn frame(value: &str, transport: &str, callback: &str) -> String {
    match transport {
        "eventsource" => format!("data: {value}\r\n\r\n"),
        "htmlfile" | "jsonp" => {
            let quoted = serde_json::to_string(value)
                .expect("Text is serializable")
                .replace('<', "\\u003c")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029");
            if transport == "htmlfile" {
                format!("<script>\np({quoted});\n</script>\r\n")
            } else {
                format!("/**/{callback}({quoted});\r\n")
            }
        }
        _ => format!("{value}\n"),
    }
}
fn text(value: String, mime: &'static str) -> Response {
    let mut response = value.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE));
    response
}
fn method(allow: &'static str) -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(allow));
    response
}
fn cors(mut response: Response, headers: &HeaderMap) -> Response {
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static(CACHE));
    if let Some(origin) = headers.get(header::ORIGIN) {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}
fn same_origin(headers: &HeaderMap, public_origin: Option<&str>) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if public_origin.is_some_and(|public| public.trim_end_matches('/') == origin) {
        return true;
    }
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|host| host.eq_ignore_ascii_case(authority))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_and_callbacks_cannot_inject_scripts() {
        assert_eq!(frame("o", "xhr", ""), "o\n");
        assert_eq!(frame("h", "eventsource", ""), "data: h\r\n\r\n");
        assert!(frame("</script>", "htmlfile", "").contains("\\u003c/script>"));
        for bad in ["x();alert(1)", "x-y", "__proto__['x']", "", "a..b"] {
            assert!(!callback_name(bad));
        }
        assert!(callback_name("_sockjs.a1"));
    }
    #[test]
    fn protocol_groups_match_upstream_names() {
        assert!(!enabled(&["streaming".into()], "xhr-streaming"));
        assert!(!enabled(&["polling".into()], "jsonp-polling"));
        assert!(enabled(&["streaming".into()], "websocket"));
        assert!(!enabled(&["iframe-eventsource".into()], "eventsource"));
    }
    #[test]
    fn authentication_cannot_be_read_from_foreign_origins() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:3838".parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost:3838".parse().unwrap());
        assert!(same_origin(&headers, None));
        headers.insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(!same_origin(&headers, None));
    }
}
