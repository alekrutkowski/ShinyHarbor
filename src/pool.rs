use crate::{
    config::{App, Config, Upstream},
    security::{epoch_ms, random_id},
};
use anyhow::{Context, Result, bail, ensure};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::Mutex as AsyncMutex,
};

pub struct Worker {
    pub id: String,
    pub origin: String,
    pub socket: Option<PathBuf>,
    pub active: AtomicUsize,
    pub sockets: AtomicUsize,
    pub shared_secret: Option<String>,
    pub sessions: Mutex<std::collections::HashSet<String>>,
    generation: Generation,
    retired: AtomicBool,
    last_used_ms: AtomicU64,
    alive: AtomicBool,
    child: Mutex<Option<Box<dyn ChildWrapper>>>,
    _runtime_directory: Option<tempfile::TempDir>,
    _socket_directory: Option<tempfile::TempDir>,
    logs: Option<Arc<WorkerLogs>>,
}
impl Worker {
    pub fn bind_session(self: &Arc<Self>, token: &str) -> SessionBinding {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(token.to_owned());
        SessionBinding {
            worker: self.clone(),
            token: token.to_owned(),
        }
    }
    fn external(origin: String, _index: usize) -> Arc<Self> {
        Arc::new(Self {
            id: format!(
                "up-{}",
                hex::encode(&Sha256::digest(origin.as_bytes())[..12])
            ),
            origin,
            socket: None,
            active: AtomicUsize::new(0),
            sockets: AtomicUsize::new(0),
            last_used_ms: AtomicU64::new(epoch_ms()),
            alive: AtomicBool::new(true),
            child: Mutex::new(None),
            shared_secret: None,
            sessions: Mutex::new(std::collections::HashSet::new()),
            generation: Generation::default(),
            retired: AtomicBool::new(false),
            _runtime_directory: None,
            _socket_directory: None,
            logs: None,
        })
    }
    fn lease(self: &Arc<Self>, websocket: bool, limit: usize) -> Result<Lease> {
        if websocket {
            let mut current = self.sockets.load(Ordering::Relaxed);
            loop {
                ensure!(
                    current < limit,
                    "This worker has reached its configured session capacity"
                );
                match self.sockets.compare_exchange_weak(
                    current,
                    current + 1,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        self.last_used_ms.store(epoch_ms(), Ordering::Relaxed);
        Ok(Lease {
            worker: self.clone(),
            websocket,
        })
    }
    fn check_alive(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = child.as_mut() {
            match c.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    if status.success()
                        && let Some(logs) = &self.logs
                    {
                        logs.success.store(true, Ordering::Release);
                    }
                    tracing::info!(worker = %self.id, %status, "R worker exited");
                    self.alive.store(false, Ordering::Release);
                    *child = None;
                }
                Err(e) => {
                    tracing::error!(worker = %self.id, error = %e, "Cannot inspect R worker");
                }
            }
        }
        self.alive.load(Ordering::Acquire)
    }
    fn stop(&self) {
        self.alive.store(false, Ordering::Release);
        if let Some(c) = self
            .child
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            && let Err(e) = c.start_kill()
        {
            tracing::warn!(worker = %self.id, error = %e, "Worker termination failed");
        }
    }
}
pub struct SessionBinding {
    worker: Arc<Worker>,
    token: String,
}
impl Drop for SessionBinding {
    fn drop(&mut self) {
        self.worker
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.token);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(child) = self
            .child
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            terminate_and_reap(child);
        }
    }
}
pub struct Lease {
    pub worker: Arc<Worker>,
    websocket: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.websocket {
            self.worker.sockets.fetch_sub(1, Ordering::AcqRel);
        }
        self.worker
            .last_used_ms
            .store(epoch_ms(), Ordering::Release);
        self.worker.active.fetch_sub(1, Ordering::AcqRel);
    }
}
pub enum Pool {
    Native {
        app: Box<App>,
        rscript: PathBuf,
        state: PathBuf,
        socket_dir: Option<PathBuf>,
        transport_limits: crate::config::SockJsLimits,
        log_tasks: tokio_util::task::TaskTracker,
        workers: Arc<Mutex<Vec<Arc<Worker>>>>,
        starting: Arc<AsyncMutex<()>>,
        startup: Arc<tokio::sync::Semaphore>,
        accepting: AtomicBool,
    },
    Upstream {
        config: Upstream,
        workers: Vec<Arc<Worker>>,
        sequence: AtomicUsize,
    },
}
#[derive(Serialize)]
pub struct PoolStatus {
    pub name: String,
    pub mount: String,
    pub runtime: &'static str,
    pub label: String,
    pub workers: usize,
    pub active_requests: usize,
    pub active_websockets: usize,
}
impl Pool {
    pub fn native(
        app: App,
        cfg: &Config,
        startup: Arc<tokio::sync::Semaphore>,
        log_tasks: tokio_util::task::TaskTracker,
    ) -> Self {
        Self::Native {
            rscript: app.rscript.clone().unwrap_or_else(|| cfg.rscript.clone()),
            app: Box::new(app),
            state: cfg.state_dir.clone(),
            socket_dir: cfg.socket_dir.clone(),
            transport_limits: cfg.sockjs.clone(),
            log_tasks,
            workers: Arc::new(Mutex::new(vec![])),
            starting: Arc::new(AsyncMutex::new(())),
            startup,
            accepting: AtomicBool::new(true),
        }
    }
    pub fn upstream(config: Upstream) -> Self {
        let workers = config
            .backends
            .iter()
            .enumerate()
            .map(|(i, o)| Worker::external(o.trim_end_matches('/').to_owned(), i))
            .collect();
        Self::Upstream {
            config,
            workers,
            sequence: AtomicUsize::new(0),
        }
    }
    pub fn same_definition(&self, candidate: &Self) -> bool {
        match (self, candidate) {
            (
                Self::Native {
                    app: a,
                    rscript: ar,
                    ..
                },
                Self::Native {
                    app: b,
                    rscript: br,
                    ..
                },
            ) => ar == br && serde_json::to_value(a).ok() == serde_json::to_value(b).ok(),
            (Self::Upstream { config: a, .. }, Self::Upstream { config: b, .. }) => {
                serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
            }
            _ => false,
        }
    }
    /// Keep session downloads and signed worker tickets routable across reloads.
    pub fn inherit_workers(&mut self, previous: &Self) -> bool {
        match (self, previous) {
            (
                Self::Native {
                    workers, starting, ..
                },
                Self::Native {
                    workers: old,
                    starting: old_starting,
                    ..
                },
            ) => {
                *workers = old.clone();
                *starting = old_starting.clone();
                true
            }
            _ => false,
        }
    }
    pub fn retire(&self) {
        if let Self::Native {
            workers, accepting, ..
        } = self
        {
            accepting.store(false, Ordering::Release);
            for worker in workers.lock().unwrap_or_else(|p| p.into_inner()).iter() {
                worker.retired.store(true, Ordering::Release);
            }
        }
    }
    pub fn draining(&self) -> bool {
        match self {
            Self::Native {
                workers, starting, ..
            } => {
                !workers.lock().unwrap_or_else(|p| p.into_inner()).is_empty()
                    || starting.try_lock().is_err()
            }
            Self::Upstream { workers, .. } => {
                workers.iter().any(|w| w.active.load(Ordering::Acquire) > 0)
            }
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Native { app, .. } => &app.name,
            Self::Upstream { config, .. } => &config.name,
        }
    }
    pub fn mount(&self) -> &str {
        match self {
            Self::Native { app, .. } => &app.mount,
            Self::Upstream { config, .. } => &config.mount,
        }
    }
    pub fn server(&self) -> usize {
        match self {
            Self::Native { app, .. } => app.server,
            Self::Upstream { config, .. } => config.server,
        }
    }
    pub fn allowed_users(&self) -> &[String] {
        match self {
            Self::Native { app, .. } => &app.allowed_users,
            Self::Upstream { config, .. } => &config.allowed_users,
        }
    }
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native { .. })
    }
    pub fn allowed_groups(&self) -> &[String] {
        match self {
            Self::Native { app, .. } => &app.allowed_groups,
            Self::Upstream { config, .. } => &config.allowed_groups,
        }
    }
    pub fn session_timeout(&self) -> Result<Option<Duration>> {
        let seconds = match self {
            Self::Native { app, .. } => crate::legacy::app_settings(app)?.session_timeout_seconds,
            Self::Upstream { .. } => 0,
        };
        Ok((seconds > 0).then(|| Duration::from_secs(seconds)))
    }
    pub fn access_policy(&self) -> Result<(Vec<String>, Vec<String>)> {
        match self {
            Self::Native { app, .. } => {
                let effective = crate::legacy::app_settings(app)?;
                Ok((effective.allowed_users, effective.allowed_groups))
            }
            Self::Upstream { config, .. } => {
                Ok((config.allowed_users.clone(), config.allowed_groups.clone()))
            }
        }
    }
    pub fn session_affinity(&self, path: &str) -> Option<String> {
        let relative = path.strip_prefix(self.mount().trim_end_matches('/'))?;
        let token = relative.strip_prefix("/session/")?.split('/').next()?;
        if let Self::Native { workers, .. } = self {
            return workers
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .find(|w| {
                    w.sessions
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(token)
                })
                .map(|w| w.id.clone());
        }
        None
    }
    pub async fn select(
        &self,
        affinity: Option<&str>,
        websocket: bool,
        new_document: bool,
    ) -> Result<Lease> {
        match self {
            Self::Upstream {
                workers, sequence, ..
            } => {
                if let Some(id) = affinity {
                    if let Some(w) = workers.iter().find(|w| w.id == id) {
                        return w.lease(websocket, usize::MAX);
                    }
                    ensure!(
                        new_document,
                        "Affinity target no longer exists; reload the application"
                    );
                }
                // Stable signed affinity is more important than retrying to an
                // arbitrary replica. A failed upgrade is never replayed elsewhere.
                let index = sequence.fetch_add(1, Ordering::Relaxed) % workers.len();
                workers[index].lease(websocket, usize::MAX)
            }
            Self::Native {
                app,
                rscript,
                state,
                socket_dir,
                transport_limits,
                log_tasks,
                workers,
                starting,
                startup,
                accepting,
            } => {
                let effective = crate::legacy::app_settings(app)?;
                let app = &effective;
                let rscript = app.rscript.as_deref().unwrap_or(rscript);
                if let Some(lease) = pick_native(app, workers, affinity, new_document, websocket)? {
                    return Ok(lease);
                }
                // Only new-worker creation is serialized. Existing workers and
                // admin status remain available while R is initializing.
                let _starting = starting.lock().await;
                if let Some(lease) = pick_native(app, workers, affinity, new_document, websocket)? {
                    return Ok(lease);
                }
                let _slot = startup.acquire().await.context("Startup limiter closed")?;
                let worker = start_worker(
                    app,
                    rscript,
                    state,
                    socket_dir.as_deref(),
                    log_tasks,
                    transport_limits,
                )
                .await?;
                let lease = worker.lease(websocket, app.sessions_per_worker)?;
                let mut all = workers.lock().unwrap_or_else(|p| p.into_inner());
                if !accepting.load(Ordering::Acquire) {
                    worker.retired.store(true, Ordering::Release);
                }
                all.push(worker);
                Ok(lease)
            }
        }
    }
    pub async fn reap(&self) {
        if let Self::Native { app, workers, .. } = self {
            let effective = crate::legacy::app_settings(app).unwrap_or_else(|_| *app.clone());
            let app = &effective;
            let mut all = workers.lock().unwrap_or_else(|p| p.into_inner());
            for w in all.iter() {
                if w.check_alive()
                    && w.active.load(Ordering::Acquire) == 0
                    && (w.retired.load(Ordering::Acquire)
                        || (app.idle_seconds > 0
                            && epoch_ms().saturating_sub(w.last_used_ms.load(Ordering::Acquire))
                                > app.idle_seconds.saturating_mul(1000)))
                {
                    tracing::info!(app = %app.name, worker = %w.id, "Stopping idle worker");
                    w.stop();
                }
            }
            // Keep killed processes until they have been reaped; do not leave zombies.
            all.retain(|w| {
                let _ = w.check_alive();
                w.child.lock().unwrap_or_else(|p| p.into_inner()).is_some()
                    || w.active.load(Ordering::Acquire) > 0
            });
        }
    }
    pub async fn stop(&self) {
        if let Self::Native { workers, .. } = self {
            let all = std::mem::take(&mut *workers.lock().unwrap_or_else(|p| p.into_inner()));
            for w in all.iter() {
                w.stop();
            }
            for _ in 0..100 {
                if all.iter().all(|w| {
                    w.check_alive();
                    w.child.lock().unwrap_or_else(|p| p.into_inner()).is_none()
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    pub async fn status(&self) -> PoolStatus {
        let refs = match self {
            Self::Native { workers, .. } => {
                workers.lock().unwrap_or_else(|p| p.into_inner()).clone()
            }
            Self::Upstream { workers, .. } => workers.clone(),
        };
        PoolStatus {
            name: self.name().into(),
            mount: self.mount().into(),
            runtime: if self.is_native() {
                "native-r"
            } else {
                "upstream"
            },
            label: match self {
                Self::Native { app, .. } => app.label.clone().unwrap_or_else(|| app.name.clone()),
                Self::Upstream { config, .. } => config.name.clone(),
            },
            workers: refs
                .iter()
                .filter(|w| w.alive.load(Ordering::Relaxed))
                .count(),
            active_requests: refs.iter().map(|w| w.active.load(Ordering::Relaxed)).sum(),
            active_websockets: refs.iter().map(|w| w.sockets.load(Ordering::Relaxed)).sum(),
        }
    }
}
// None requests a new worker. This function never waits for R startup.
fn pick_native(
    app: &App,
    workers: &Mutex<Vec<Arc<Worker>>>,
    affinity: Option<&str>,
    new_document: bool,
    websocket: bool,
) -> Result<Option<Lease>> {
    let mut all = workers.lock().unwrap_or_else(|p| p.into_inner());
    let generation = restart_stamp(app);
    for worker in all.iter() {
        worker.check_alive();
        if worker.generation != generation {
            worker.retired.store(true, Ordering::Release);
        }
    }
    all.retain(|w| {
        w.alive.load(Ordering::Acquire)
            || w.active.load(Ordering::Acquire) > 0
            || w.child.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    });
    let live = all
        .iter()
        .filter(|w| w.alive.load(Ordering::Acquire) && !w.retired.load(Ordering::Acquire))
        .count();
    let trigger = ((app.sessions_per_worker as f64 * app.load_factor).floor() as usize).max(1);
    if let Some(id) = affinity {
        if let Some(worker) = all.iter().find(|w| {
            w.id == id
                && w.alive.load(Ordering::Acquire)
                && (!new_document || !w.retired.load(Ordering::Acquire))
                && (!new_document || w.sockets.load(Ordering::Acquire) < app.sessions_per_worker)
                && (!new_document
                    || live >= app.max_workers
                    || w.active.load(Ordering::Acquire) < trigger)
        }) {
            return worker.lease(websocket, app.sessions_per_worker).map(Some);
        }
        ensure!(
            new_document,
            "R worker has exited; reload the application to create a new session"
        );
    }
    let candidate = all
        .iter()
        .filter(|w| w.alive.load(Ordering::Acquire) && !w.retired.load(Ordering::Acquire))
        .min_by_key(|w| {
            (
                w.sockets.load(Ordering::Relaxed),
                w.active.load(Ordering::Relaxed),
            )
        })
        .cloned();
    let should_start = candidate.as_ref().is_none_or(|w| {
        w.active
            .load(Ordering::Relaxed)
            .max(w.sockets.load(Ordering::Relaxed))
            >= trigger
    });
    if should_start && live < app.max_workers {
        return Ok(None);
    }
    candidate
        .context("No healthy R worker is available")?
        .lease(websocket, app.sessions_per_worker)
        .map(Some)
}
#[derive(Default, Clone, Copy, PartialEq)]
struct Generation {
    restart: Option<SystemTime>,
    configuration: Option<SystemTime>,
}
fn restart_stamp(app: &App) -> Generation {
    let dir = if app.path.is_dir() {
        app.path.as_path()
    } else {
        app.path.parent().unwrap_or(&app.path)
    };
    Generation {
        restart: std::fs::metadata(dir.join("restart.txt"))
            .ok()
            .and_then(|m| m.modified().ok()),
        configuration: app
            .allow_app_override
            .then(|| {
                std::fs::metadata(dir.join(".shiny_app.conf"))
                    .ok()
                    .and_then(|m| m.modified().ok())
            })
            .flatten(),
    }
}
// Own the child during startup. Cancelling a request or timing out readiness must
// not orphan a process group/job object.
struct Starting(Option<Box<dyn ChildWrapper>>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            terminate_and_reap(child);
        }
    }
}
fn terminate_and_reap(mut child: Box<dyn ChildWrapper>) {
    let _ = child.start_kill();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            let _ = child.wait().await;
        });
    }
}
async fn start_worker(
    app: &App,
    rscript: &Path,
    state: &Path,
    socket_root: Option<&Path>,
    log_tasks: &tokio_util::task::TaskTracker,
    transport_limits: &crate::config::SockJsLimits,
) -> Result<Arc<Worker>> {
    let identity = crate::identity::resolve(app)?;
    let socket_directory = socket_root
        .map(|root| -> Result<tempfile::TempDir> {
            let directory = tempfile::Builder::new()
                .prefix("w-")
                .rand_bytes(8)
                .tempdir_in(root)?;
            crate::security::private_dir(directory.path())?;
            if let Some(identity) = &identity {
                identity.own(directory.path())?;
            }
            ensure!(
                directory.path().join("r.sock").as_os_str().len() <= 103,
                "socket_dir path is too long for a portable Unix domain socket"
            );
            Ok(directory)
        })
        .transpose()?;
    let socket = socket_directory.as_ref().map(|d| d.path().join("r.sock"));
    let runtime_directory = if identity.is_some() {
        Some(
            tempfile::Builder::new()
                .prefix("shinyharbor-worker-")
                .tempdir()?,
        )
    } else {
        None
    };
    let runtime_root = runtime_directory.as_ref().map_or(state, |dir| dir.path());
    if let Some(identity) = &identity {
        crate::security::private_dir(runtime_root)?;
        for (name, content) in [
            ("bootstrap.R", include_str!("../r/bootstrap.R")),
            ("runtime.R", include_str!("../r/runtime.R")),
        ] {
            let path = runtime_root.join(name);
            std::fs::write(&path, content)?;
            identity.own(&path)?;
        }
        identity.own(runtime_root)?;
    }
    let id = random_id();
    let ticket =
        crate::security::Secret::load_or_create(&state.join("secret.key"))?.sign(&app.name, &id);
    let shared_secret = format!("{}{}", random_id(), random_id());
    let generation = restart_stamp(app);
    let ready = if identity.is_some() {
        runtime_root.join(format!("{id}.ready"))
    } else {
        state.join("run").join(format!("{id}.ready"))
    };
    let bootstrap = runtime_root.join("bootstrap.R");
    let working = if app.path.is_dir() {
        app.path.as_path()
    } else {
        app.path
            .parent()
            .context("Application path has no parent")?
    };
    let bookmarks = match (&app.bookmark_state_dir, &identity) {
        (Some(path), _) => path.clone(),
        (None, Some(identity)) => identity.bookmarks(state)?,
        (None, None) => state.join("bookmarks"),
    };
    let logs = app.log_dir.clone().unwrap_or_else(|| state.join("logs"));
    if let Some(identity) = &identity {
        if app.bookmark_state_dir.is_some() {
            // The configured shared root stays service-owned. R creates its
            // per-account children inside a pre-created private namespace.
            identity.prepare_bookmarks(&bookmarks)?;
        }
    } else {
        crate::security::private_dir(&bookmarks)?;
    }
    let user_log = match (app.log_as_user, &identity) {
        (true, Some(identity)) => Some(
            crate::user_log::UserLog::start(
                &logs.join(format!("{}-{id}", app.name)),
                &app.log_file_mode,
                app.preserve_logs,
                identity,
                log_tasks,
            )
            .await?,
        ),
        _ => {
            crate::security::private_dir(&logs)?;
            None
        }
    };
    // Positional arguments keep paths, quotes and metacharacters out of shell
    // code. Identity::apply drops credentials before Bash reads any profiles.
    let mut arguments: Vec<std::ffi::OsString> = app
        .supervisor
        .iter()
        .map(std::ffi::OsString::from)
        .collect();
    arguments.push(rscript.as_os_str().to_owned());
    arguments.extend(["--no-save".into(), "--no-restore".into()]);
    if rscript.file_stem().is_some_and(|name| name == "R") {
        arguments.push("--file".into());
    }
    arguments.push(bootstrap.into_os_string());
    let executable = if app.login_shell {
        std::ffi::OsStr::new("/bin/bash")
    } else {
        arguments[0].as_os_str()
    };
    let mut cmd = CommandWrap::with_new(executable, |cmd| {
        if app.login_shell {
            cmd.args([
                "--login",
                "-c",
                "cd -- \"$1\" && shift && exec \"$@\"",
                "shinyharbor-worker",
            ])
            .arg(working)
            .args(&arguments);
        } else {
            cmd.args(&arguments[1..]);
        }
        cmd.current_dir(working)
            .envs(&app.environment)
            .env("HARBOR_APP", &app.path)
            .env("HARBOR_SERVER_VERSION", env!("CARGO_PKG_VERSION"))
            .env("HARBOR_READY", &ready)
            .env("HARBOR_SOCKET", socket.as_deref().unwrap_or(Path::new("")))
            .env("HARBOR_RUNTIME", runtime_root.join("runtime.R"))
            .env("HARBOR_SHARED_SECRET", &shared_secret)
            .env("HARBOR_BOOKMARKS", &bookmarks)
            .env(
                "HARBOR_GOOGLE_ANALYTICS_ID",
                app.google_analytics_id.as_deref().unwrap_or(""),
            )
            .env("SHINY_SHARED_SECRET", &shared_secret)
            .env(
                "HARBOR_SANITIZE_ERRORS",
                if app.sanitize_errors { "true" } else { "false" },
            )
            .env("SHINY_APP", &app.path)
            .env("WORKER_ID", &id)
            .env("HARBOR_WORKER_TICKET", &ticket)
            .env(
                "HARBOR_SEND_BUFFER_BYTES",
                transport_limits.max_replay_bytes.to_string(),
            )
            .env(
                "HARBOR_SEND_BUFFER_MESSAGES",
                transport_limits.max_replay_messages.to_string(),
            )
            .env(
                "HARBOR_RECONNECT",
                if app.reconnect { "true" } else { "false" },
            )
            .env(
                "HARBOR_DISABLE_PROTOCOLS",
                serde_json::to_string(&app.disable_protocols)
                    .expect("Protocol list is serializable"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(identity) = &identity {
            identity.apply(cmd);
        }
    });
    cmd.wrap(KillOnDrop);
    #[cfg(unix)]
    cmd.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    cmd.wrap(process_wrap::tokio::JobObject);
    let mut child = Starting(Some(cmd.spawn().with_context(|| {
        format!(
            "Cannot start {} for {}. Check rscript, supervisor and /bin/bash if login_shell is enabled.",
            rscript.display(),
            app.name
        )
    })?));
    let logs = Arc::new(WorkerLogs {
        paths: vec![
            logs.join(format!("{}-{id}-out.log", app.name)),
            logs.join(format!("{}-{id}-err.log", app.name)),
        ],
        preserve: app.preserve_logs,
        success: AtomicBool::new(false),
        user_log,
    });
    let mode = crate::config::log_mode(&app.log_file_mode)?;
    let proc = child.0.as_mut().context("Missing child")?;
    if let Some(out) = proc.stdout().take() {
        log_tasks.spawn(drain_log(out, logs.paths[0].clone(), mode, logs.clone()));
    }
    if let Some(err) = proc.stderr().take() {
        log_tasks.spawn(drain_log(err, logs.paths[1].clone(), mode, logs.clone()));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(app.startup_seconds);
    loop {
        if let Some(status) = child.0.as_mut().context("Missing child")?.try_wait()? {
            let _ = tokio::fs::remove_file(&ready).await;
            bail!(
                "Application {} exited during startup ({status}). See {}-{}-err.log",
                app.name,
                app.name,
                id
            );
        }
        let origin = if let Some(socket) = &socket {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                std::fs::symlink_metadata(socket)
                    .ok()
                    .filter(|m| m.file_type().is_socket())
                    .map(|_| "http://127.0.0.1".to_owned())
            }
            #[cfg(not(unix))]
            {
                let _ = socket;
                None
            }
        } else if let Ok(text) = tokio::fs::read_to_string(&ready).await {
            let url = url::Url::parse(text.trim()).context("Invalid R readiness URL")?;
            ensure!(
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some_and(|p| p > 0),
                "R must bind to loopback"
            );
            Some(url.origin().ascii_serialization())
        } else {
            None
        };
        if let Some(origin) = origin {
            let _ = tokio::fs::remove_file(&ready).await;
            tracing::info!(app = %app.name, worker = %id, "R application is ready");
            return Ok(Arc::new(Worker {
                id,
                origin,
                socket,
                active: AtomicUsize::new(0),
                sockets: AtomicUsize::new(0),
                last_used_ms: AtomicU64::new(epoch_ms()),
                alive: AtomicBool::new(true),
                shared_secret: Some(shared_secret),
                sessions: Mutex::new(std::collections::HashSet::new()),
                generation,
                retired: AtomicBool::new(false),
                child: Mutex::new(child.0.take()),
                _runtime_directory: runtime_directory,
                _socket_directory: socket_directory,
                logs: Some(logs),
            }));
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = tokio::fs::remove_file(&ready).await;
            bail!(
                "Application {} did not become ready within {} seconds; inspect its worker log",
                app.name,
                app.startup_seconds
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
struct WorkerLogs {
    paths: Vec<PathBuf>,
    preserve: bool,
    success: AtomicBool,
    user_log: Option<crate::user_log::UserLog>,
}
impl Drop for WorkerLogs {
    fn drop(&mut self) {
        if let Some(log) = &self.user_log {
            log.finish(!self.preserve && self.success.load(Ordering::Acquire));
            return;
        }
        if !self.preserve && self.success.load(Ordering::Acquire) {
            for path in &self.paths {
                let _ = std::fs::remove_file(path);
                let _ = std::fs::remove_file(path.with_extension("log.1"));
            }
        }
    }
}
async fn open_log(path: &Path, mode: u32) -> Result<tokio::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(tokio::fs::File::from_std(file))
}
async fn drain_log<R: AsyncRead + Unpin>(
    mut input: R,
    path: PathBuf,
    mode: u32,
    logs: Arc<WorkerLogs>,
) {
    let relay = std::env::var_os("SHINY_LOG_STDERR").is_some_and(|v| !v.is_empty());
    if let Some(log) = &logs.user_log {
        let channel = u8::from(path == logs.paths[1]);
        let mut buf = [0; 16 * 1024];
        let mut writable = true;
        while let Ok(n) = input.read(&mut buf).await {
            if n == 0 {
                break;
            }
            if writable && let Err(error) = log.write(channel, &buf[..n]).await {
                tracing::error!(%error, "Worker-owned log output unavailable");
                writable = false;
            }
            if relay {
                let _ = tokio::io::stderr().write_all(&buf[..n]).await;
            }
        }
        return;
    }
    // Bounded memory even for a single multi-megabyte line; bounded log files.
    let result: Result<()> = async {
        let mut file = open_log(&path, mode).await?;
        if logs.preserve {
            // Persist retention intent across config changes and server restarts.
            let marker = path.with_extension("log.keep");
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(marker)?;
        }
        let mut size = 0_u64;
        let mut buf = [0_u8; 16 * 1024];
        loop {
            let n = input.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            if size + n as u64 > 10 * 1024 * 1024 {
                file.flush().await?;
                drop(file);
                let old = path.with_extension("log.1");
                let _ = tokio::fs::remove_file(&old).await;
                tokio::fs::rename(&path, old).await?;
                file = open_log(&path, mode).await?;
                size = 0;
            }
            file.write_all(&buf[..n]).await?;
            if relay {
                tokio::io::stderr().write_all(&buf[..n]).await?;
            }
            size += n as u64;
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::error!(%error, "Application log write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_drains_old_sessions_and_routes_new_documents_to_new_generation() {
        let temp = tempfile::tempdir().unwrap();
        let app = App {
            path: temp.path().to_owned(),
            max_workers: 1,
            ..App::default()
        };
        let old = Worker::external("http://127.0.0.1:1".into(), 0);
        let workers = Mutex::new(vec![old.clone()]);
        let active = old.lease(true, 10).unwrap();
        std::fs::write(temp.path().join("restart.txt"), "").unwrap();
        assert!(
            pick_native(&app, &workers, Some(&old.id), true, false)
                .unwrap()
                .is_none()
        );
        assert!(old.alive.load(Ordering::Acquire));
        let download = pick_native(&app, &workers, Some(&old.id), false, false)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&download.worker, &old));
        assert_eq!(old.sockets.load(Ordering::Relaxed), 1);
        drop(download);
        drop(active);
        assert_eq!(old.active.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn session_capacity_recovers_after_disconnect() {
        let worker = Worker::external("http://127.0.0.1:1".into(), 0);
        let session = worker.lease(true, 1).unwrap();
        assert!(worker.lease(true, 1).is_err());
        drop(session);
        assert!(worker.lease(true, 1).is_ok());
    }
}
