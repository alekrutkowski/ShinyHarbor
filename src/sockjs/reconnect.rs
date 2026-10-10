//! Original implementation of Shiny Server's numbered-message and multiplex
//! wire contracts. A logical session outlives individual SockJS transports.
use super::*;
use anyhow::{Context, Result, ensure};
use std::collections::VecDeque;
use tokio::sync::oneshot;

const GRACE: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(super) struct Registry {
    entries: Mutex<HashMap<Key, Arc<Handle>>>,
}
struct Handle {
    owner: Option<Identity>,
    attach: mpsc::Sender<Transport>,
    stop: CancellationToken,
}
impl Registry {
    pub(super) fn reap(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|_, handle| !handle.stop.is_cancelled());
    }
}
struct Transport {
    session: Arc<Session>,
    output: mpsc::Sender<Queued>,
    input: mpsc::Receiver<Batch>,
    resume: bool,
    ready: Option<oneshot::Sender<()>>,
}
impl Transport {
    fn write(&self, message: &str) {
        if queue(&self.output, data_frame(message), &self.session.budget).is_err() {
            self.session.stop.cancel();
        }
    }
    fn close(&self, code: u16, reason: &str) {
        let frame = format!("c{}", serde_json::json!([code, reason]));
        let _ = queue(&self.output, frame, &self.session.budget);
        self.session.done.store(true, Ordering::Release);
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.session.done.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn create(
    state: &Arc<AppState>,
    pool: &Arc<Pool>,
    user: Option<Identity>,
    peer: SocketAddr,
    headers: &HeaderMap,
    worker: Option<&str>,
    id: &str,
    existing: bool,
    enabled: bool,
) -> Result<Arc<Session>> {
    let (session, output, input) = session_channels(user.clone(), &state.sockjs.budget)?;
    let (ready, waiting) = oneshot::channel();
    let transport = Transport {
        session: session.clone(),
        output,
        input,
        resume: existing,
        ready: Some(ready),
    };
    if id != "none" && !enabled {
        transport.close(4500, "Session reconnect is disabled");
        return Ok(session);
    }
    let key = (
        pool.name().to_owned(),
        if id == "none" {
            security::random_id()
        } else {
            id.to_owned()
        },
    );
    let handle = {
        let mut all = state
            .sockjs
            .reconnect
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(handle) = all.get(&key).filter(|h| !h.stop.is_cancelled()) {
            ensure!(handle.owner == user, "Reconnect identity mismatch");
            handle.clone()
        } else {
            if existing {
                transport.close(4705, "Your session could not be resumed on the server.");
                return Ok(session);
            }
            ensure!(
                all.len() < state.config.max_inflight.min(4096),
                "Reconnect session limit reached"
            );
            let (attach, incoming) = mpsc::channel(4);
            let handle = Arc::new(Handle {
                owner: user,
                attach,
                stop: CancellationToken::new(),
            });
            state.tunnels.spawn(run(
                state.clone(),
                pool.clone(),
                headers.clone(),
                peer,
                worker.map(str::to_owned),
                handle.stop.clone(),
                incoming,
                id != "none",
            ));
            all.insert(key, handle.clone());
            handle
        }
    };
    handle
        .attach
        .try_send(transport)
        .map_err(|_| anyhow::anyhow!("Reconnect already in progress"))?;
    tokio::time::timeout(Duration::from_secs(30), waiting).await??;
    Ok(session)
}

/// ACK and CONTINUE carry the first message not received, in uppercase hex.
#[derive(Default)]
struct Protocol {
    received: u64,
    sent: u64,
    first: u64,
    bytes: usize,
    waiting: bool,
    replay: VecDeque<Queued>,
}
impl Protocol {
    fn acknowledge(&mut self, next: u64) -> Result<()> {
        ensure!(
            next >= self.first && next <= self.sent,
            "Acknowledgment outside replay window"
        );
        while self.first < next {
            let old = self.replay.pop_front().context("Missing replay message")?;
            self.bytes -= old.text.len();
            self.first += 1;
        }
        Ok(())
    }
    fn output(
        &mut self,
        value: &str,
        budget: &Arc<Semaphore>,
        limits: &crate::config::SockJsLimits,
    ) -> Result<String> {
        let value = format!("{:X}#{value}", self.sent);
        ensure!(
            self.bytes + value.len() <= limits.max_replay_bytes
                && self.replay.len() < limits.max_replay_messages,
            "Reconnect replay budget exceeded"
        );
        let permit = budget
            .clone()
            .try_acquire_many_owned(u32::try_from(value.len() + std::mem::size_of::<Queued>())?)?;
        self.bytes += value.len();
        self.sent = self
            .sent
            .checked_add(1)
            .context("Message sequence exhausted")?;
        self.replay.push_back(Queued {
            text: value.clone(),
            _budget: permit,
        });
        Ok(value)
    }
    fn receive<'a>(
        &mut self,
        value: &'a str,
        transport: Option<&Transport>,
    ) -> Result<Option<&'a str>> {
        if let Some(id) = value.strip_prefix("ACK ") {
            self.acknowledge(number(id)?)?;
            return Ok(None);
        }
        if self.waiting {
            let id = value
                .strip_prefix("CONTINUE ")
                .context("Expected CONTINUE")?;
            self.acknowledge(number(id)?)?;
            self.waiting = false;
            if let Some(transport) = transport {
                for entry in &self.replay {
                    transport.write(&entry.text);
                }
            }
            return Ok(None);
        }
        let (tag, message) = value.split_once('#').context("Missing sequence number")?;
        let sequence = number(tag)?;
        ensure!(sequence <= self.received, "Out of order client message");
        if sequence < self.received {
            return Ok(None);
        } // Exactly once replay.
        self.received = self
            .received
            .checked_add(1)
            .context("Message sequence exhausted")?;
        if let Some(transport) = transport {
            transport.write(&format!("ACK {:X}", self.received));
        }
        Ok(Some(message))
    }
}
fn number(value: &str) -> Result<u64> {
    ensure!(
        !value.is_empty()
            && value.len() <= 13
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)),
        "Invalid hexadecimal sequence"
    );
    Ok(u64::from_str_radix(value, 16)?)
}

struct Channel {
    generation: u64,
    input: mpsc::Sender<Queued>,
    stop: CancellationToken,
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
struct ChannelMessage {
    channel: String,
    generation: u64,
    message: Queued,
    closed: bool,
}

#[allow(clippy::too_many_arguments)]
async fn run(
    state: Arc<AppState>,
    pool: Arc<Pool>,
    headers: HeaderMap,
    peer: SocketAddr,
    worker: Option<String>,
    stop: CancellationToken,
    mut attaches: mpsc::Receiver<Transport>,
    robust: bool,
) {
    let _cancel = stop.clone().drop_guard();
    let mut protocol = Protocol::default();
    let mut transport: Option<Transport> = None;
    let mut channels = HashMap::<String, Channel>::new();
    let mut generation = 0u64;
    let (events, mut messages) = mpsc::channel::<ChannelMessage>(32);
    let mut deadline = tokio::time::Instant::now() + GRACE;
    let mut attached_before = false;
    let mut final_code = 4503;
    let mut final_reason = "Application session ended";
    loop {
        let physical_stop = transport
            .as_ref()
            .map(|t| t.session.stop.clone())
            .unwrap_or_default();
        tokio::select! {
            biased;
            _ = state.force_stop.cancelled() => { final_code = 4702; final_reason = "Server shutting down"; break; },
            _ = stop.cancelled() => break,
            incoming = attaches.recv() => {
                let Some(mut next) = incoming else { break; };
                if !next.resume && attached_before && protocol.received > 0 {
                    next.close(4705, "Session identifier already in use");
                    if let Some(ready) = next.ready.take() { let _ = ready.send(()); }
                    continue;
                }
                if let Some(old) = transport.take() { old.close(4506, "Connection replaced"); }
                if next.resume {
                    protocol.waiting = true;
                    next.write(&format!("CONTINUE {:X}", protocol.received));
                } else if attached_before {
                    for message in &protocol.replay { next.write(&message.text); }
                }
                if let Some(ready) = next.ready.take() { let _ = ready.send(()); }
                transport = Some(next);
                attached_before = true;
            },
            _ = physical_stop.cancelled(), if transport.is_some() => {
                transport = None;
                if !robust { break; }
                deadline = tokio::time::Instant::now() + GRACE;
            },
            _ = tokio::time::sleep_until(deadline), if transport.is_none() => break,
            batch = async {
                match transport.as_mut() { Some(t) => t.input.recv().await, None => std::future::pending().await }
            } => {
                let Some(batch) = batch else { break; };
                let mut invalid = false;
                for message in batch.messages {
                    let payload = if robust {
                        match protocol.receive(&message, transport.as_ref()) {
                            Ok(Some(data)) => data, Ok(None) => continue, Err(_) => { invalid = true; break; }
                        }
                    } else { &message };
                    let mut fields = payload.splitn(3, '|');
                    let id = fields.next().unwrap_or("");
                    let method = fields.next().unwrap_or("");
                    let data = fields.next();
                    if id.is_empty() || id.len() > 16 || !id.bytes().all(|b| b.is_ascii_digit()) || data.is_none() {
                        invalid = true; break;
                    }
                    let data = data.unwrap_or("");
                    match method {
                        "o" if !channels.contains_key(id) => {
                            if channels.len() >= 64 { invalid = true; break; }
                            let (input, receiver) = mpsc::channel(8);
                            generation += 1;
                            let channel = Channel { generation, input, stop: stop.child_token() };
                            state.tunnels.spawn(channel_task(state.clone(), pool.clone(), headers.clone(), peer,
                                worker.clone(), id.to_owned(), generation, robust, data.to_owned(), channel.stop.clone(), receiver, events.clone()));
                            channels.insert(id.to_owned(), channel);
                        },
                        "o" => {},
                        "c" => {
                            if serde_json::from_str::<serde_json::Value>(data).is_err() { invalid = true; break; }
                            channels.remove(id);
                        },
                        "m" => if let Some(channel) = channels.get(id)
                            && queue(&channel.input, data.to_owned(), &state.sockjs.budget).is_err() { invalid = true; break; },
                        _ => { invalid = true; break; },
                    }
                }
                if invalid { final_code = 4704; final_reason = "Invalid session protocol"; break; }
            },
            event = messages.recv() => {
                let Some(event) = event else { break; };
                if channels.get(&event.channel).is_none_or(|c| c.generation != event.generation) { continue; }
                if event.closed { channels.remove(&event.channel); }
                let value = if robust {
                    match protocol.output(&event.message.text, &state.sockjs.budget, &state.config.sockjs) {
                        Ok(value) => value, Err(_) => { final_code = 4704; final_reason = "Session replay limit exceeded"; break; }
                    }
                } else { event.message.text };
                if !protocol.waiting && let Some(transport) = &transport { transport.write(&value); }
            }
        }
    }
    if let Some(transport) = transport {
        transport.close(final_code, final_reason);
    }
}

async fn channel_pool(state: &AppState, parent: &Arc<Pool>, path: &str) -> Result<Arc<Pool>> {
    let base = url::Url::parse(&format!(
        "http://shinyharbor.invalid{}/",
        parent.mount().trim_end_matches('/')
    ))?;
    let url = base.join(path)?;
    ensure!(url.origin() == base.origin(), "Multiplex URL must be local");
    let path =
        crate::config::canonical_path(url.path()).context("Invalid multiplex URL encoding")?;
    let pools = state.pools();
    let mut found = pools
        .iter()
        .filter(|p| p.server() == parent.server() && crate::config::matches_mount(&path, p.mount()))
        .max_by_key(|p| p.mount().len())
        .cloned();
    if let Some(site) = state
        .config
        .sites
        .iter()
        .filter(|s| {
            s.app_defaults.server == parent.server()
                && crate::config::matches_mount(&path, &s.mount)
        })
        .max_by_key(|s| s.mount.len())
        .filter(|s| {
            found
                .as_ref()
                .is_none_or(|p| s.mount.len() > p.mount().len())
        })
    {
        let crate::site::Target::App(app) = crate::site::resolve(site, &path, &state.private_paths)
            .await
            .map_err(|_| anyhow::anyhow!("Application not found"))?
        else {
            anyhow::bail!("Not an application");
        };
        let mut all = state.pools.write().unwrap_or_else(|p| p.into_inner());
        found = if let Some(pool) = all
            .iter()
            .find(|p| p.server() == parent.server() && p.mount() == app.mount)
        {
            Some(pool.clone())
        } else {
            ensure!(
                all.len() < state.config.apps.len() + state.config.upstreams.len() + 4096,
                "Application limit reached"
            );
            let pool = Arc::new(state.native_pool(*app));
            all.push(pool.clone());
            Some(pool)
        };
    }
    found
        .filter(|p| p.is_native())
        .context("No native application at multiplex URL")
}

#[allow(clippy::too_many_arguments)]
async fn channel_task(
    state: Arc<AppState>,
    parent: Arc<Pool>,
    headers: HeaderMap,
    peer: SocketAddr,
    worker: Option<String>,
    id: String,
    generation: u64,
    robust: bool,
    path: String,
    stop: CancellationToken,
    mut input: mpsc::Receiver<Queued>,
    events: mpsc::Sender<ChannelMessage>,
) {
    let result: Result<()> = async {
        let state = state.current();
        let pool = channel_pool(&state, &parent, &path).await?;
        if let Pool::Native { app, .. } = pool.as_ref() {
            ensure!(!robust || crate::legacy::app_settings(app)?.reconnect, "Session reconnect is disabled for this application");
        }
        let (allowed_users, allowed_groups, ignore_case) = pool.access_policy()?;
        if !allowed_users.is_empty() || !allowed_groups.is_empty() {
            let tls = state.config.servers.get(parent.server()).map_or(&state.config.tls, |s| state.config.listener_tls(s.listen));
            ensure!(peer.ip().is_loopback() || !matches!(tls, crate::config::Tls::Off {}) || state.config.public_origin.is_some(), "Authentication requires TLS");
        }
        let user = state.auth.authorize_request(&headers, peer.ip(), &allowed_users, &allowed_groups, ignore_case).await.map_err(|_| anyhow::anyhow!("Access denied"))?;
        let worker = if pool.name() == parent.name() { worker.as_deref() } else { None };
        let connection = connect_backend(&state, &pool, user.as_ref(), peer, &headers, worker);
        let (lease, mut backend) = tokio::select! { _ = stop.cancelled() => return Ok(()), result = connection => result? };
        let mut binding = None;
        let started = Instant::now();
        let idle = pool.session_timeout()?;
        let mut last_data = tokio::time::Instant::now();
        let ip = security::client_ip(peer.ip(), &headers, &state.config.trusted_proxies);
        let mut event = state.analytics.event(ip, &headers, pool.name());
        if let Some(e) = event.as_mut() { e.request = 1; e.websocket = 1; }
        state.analytics.record(event.clone());
        if let Some(e) = event.as_mut() { e.request = 0; e.websocket = 0; }
        loop {
            tokio::select! {
                _ = idle_elapsed(last_data, idle) => break,
                _ = stop.cancelled() => break,
                _ = state.force_stop.cancelled() => break,
                incoming = input.recv() => match incoming {
                    Some(message) => {
                        last_data = tokio::time::Instant::now();
                        if let Some(e) = event.as_mut() { e.bytes_in += message.text.len() as u64; }
                        if !tokio::time::timeout(Duration::from_secs(30), backend.send(Message::Text(message.text.into()))).await.is_ok_and(|r| r.is_ok()) { break; }
                    },
                    None => break,
                },
                incoming = backend.next() => match incoming {
                    Some(Ok(Message::Text(value))) => {
                        last_data = tokio::time::Instant::now();
                        if binding.is_none()
                            && let Ok(message) = serde_json::from_str::<serde_json::Value>(&value)
                            && let Some(token) = message.get("config").and_then(|v| v.get("sessionId")).and_then(|v| v.as_str()).filter(|s| identifier(s)) {
                            binding = Some(lease.worker.bind_session(token));
                        }
                        if let Some(e) = event.as_mut() { e.bytes_out += value.len() as u64; }
                        if channel_message(&events, &state.sockjs.budget, &id, generation, format!("{id}|m|{value}"), false).is_err() { break; }
                    },
                    Some(Ok(Message::Ping(_))) => { if backend.flush().await.is_err() { break; } },
                    Some(Ok(Message::Pong(_))) => {},
                    _ => break,
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), backend.close(None)).await;
        if let Some(e) = event.as_mut() { e.websocket_ms = started.elapsed().as_millis() as u64; }
        state.analytics.record(event);
        Ok(())
    }.await;
    let code = if let Err(error) = result {
        tracing::warn!(%error, "Multiplex application connection failed");
        4501
    } else {
        4503
    };
    let message = format!(
        "{id}|c|{}",
        serde_json::json!({"code":code,"reason":"Application connection closed"})
    );
    let _ = channel_message(
        &events,
        &state.sockjs.budget,
        &id,
        generation,
        message,
        true,
    );
}
fn channel_message(
    tx: &mpsc::Sender<ChannelMessage>,
    budget: &Arc<Semaphore>,
    id: &str,
    generation: u64,
    text: String,
    closed: bool,
) -> Result<()> {
    let permit = budget
        .clone()
        .try_acquire_many_owned(u32::try_from(text.len())?)?;
    tx.try_send(ChannelMessage {
        channel: id.to_owned(),
        generation,
        message: Queued {
            text,
            _budget: permit,
        },
        closed,
    })
    .map_err(|_| anyhow::anyhow!("Session receiver too slow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_replay_and_budgets() {
        let budget = Arc::new(Semaphore::new(1024));
        let limits = crate::config::SockJsLimits::default();
        let mut p = Protocol::default();
        assert_eq!(
            p.output("0|m|hello", &budget, &limits).unwrap(),
            "0#0|m|hello"
        );
        assert_eq!(
            p.output("0|m|world", &budget, &limits).unwrap(),
            "1#0|m|world"
        );
        p.receive("ACK 1", None).unwrap();
        p.waiting = true;
        assert!(p.receive("0#data", None).is_err());
        p.receive("CONTINUE 1", None).unwrap();
        assert_eq!(p.replay.front().unwrap().text, "1#0|m|world");
        assert_eq!(p.receive("0#data", None).unwrap(), Some("data"));
        assert_eq!(p.receive("0#data", None).unwrap(), None);
        assert!(p.receive("2#lost", None).is_err());
        assert!(p.receive("ACK 3", None).is_err());
        p.receive("ACK 2", None).unwrap();
        assert_eq!(budget.available_permits(), 1024);
    }
    #[test]
    fn configured_replay_limits_recover_after_acknowledgment() {
        let budget = Arc::new(Semaphore::new(1024));
        let mut limits = crate::config::SockJsLimits {
            max_replay_bytes: 16,
            max_replay_messages: 1,
            ..Default::default()
        };
        let mut protocol = Protocol::default();
        protocol.output("hello", &budget, &limits).unwrap();
        assert!(protocol.output("another", &budget, &limits).is_err());
        protocol.acknowledge(1).unwrap();
        assert_eq!(budget.available_permits(), 1024);
        limits.max_replay_messages = 8;
        assert!(
            protocol
                .output("this is larger than sixteen bytes", &budget, &limits)
                .is_err()
        );
        assert_eq!(budget.available_permits(), 1024);
        protocol.output("next", &budget, &limits).unwrap();
    }
}
