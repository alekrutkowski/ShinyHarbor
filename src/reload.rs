//! Atomic application configuration snapshots, with draining worker generations.
use crate::{auth::Auth, config::Config, pool::Pool, proxy::AppState};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, RwLock},
};

pub struct Control {
    current: RwLock<Arc<AppState>>,
    pub(crate) path: Option<PathBuf>,
    retired: Mutex<Vec<Arc<Pool>>>,
    pub(crate) reloading: tokio::sync::Mutex<()>,
}
impl Control {
    pub fn new(state: Arc<AppState>, path: Option<PathBuf>) -> Arc<Self> {
        let control = Arc::new(Self {
            current: RwLock::new(state.clone()),
            path,
            retired: Mutex::new(vec![]),
            reloading: tokio::sync::Mutex::new(()),
        });
        let _ = state.control.set(Arc::downgrade(&control));
        control
    }
    pub fn snapshot(&self) -> Arc<AppState> {
        self.current
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn pools(&self) -> Vec<Arc<Pool>> {
        let mut pools = self.snapshot().pools();
        pools.extend(
            self.retired
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .cloned(),
        );
        pools
    }
    pub fn prune_retired(&self) {
        self.retired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|p| p.draining());
    }
    pub fn inherit_discovered(&self, pool: &mut Pool) {
        let mut retired = self.retired.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(index) = retired.iter().position(|p| {
            p.name() == pool.name() && p.mount() == pool.mount() && p.server() == pool.server()
        }) && pool.inherit_workers(&retired[index])
        {
            retired.remove(index);
        }
    }
    pub async fn reload(self: &Arc<Self>) -> Result<()> {
        let _exclusive = self.reloading.lock().await;
        let path = self
            .path
            .clone()
            .context("Reload requires a configuration file")?;
        let config = tokio::task::spawn_blocking(move || Config::load(&path)).await??;
        self.apply(config).await
    }
    pub(crate) async fn apply(self: &Arc<Self>, config: Config) -> Result<()> {
        let previous = self.snapshot();
        ensure!(
            fixed_settings(&config)? == fixed_settings(&previous.config)?,
            "Listener, TLS, state, analytics and global transport settings require a service restart; the previous configuration remains active"
        );
        let old = previous.pools();
        let mut retired = vec![];
        let mut replaced = vec![];
        let mut inherited = Vec::new();
        let candidates = config
            .apps
            .iter()
            .cloned()
            .map(|a| {
                Pool::native(
                    a,
                    &config,
                    previous.startup.clone(),
                    previous.log_tasks.clone(),
                )
            })
            .chain(config.upstreams.iter().cloned().map(Pool::upstream));
        let mut pools = Vec::new();
        for mut candidate in candidates {
            if let Some(existing) = old.iter().find(|p| {
                p.name() == candidate.name()
                    && p.mount() == candidate.mount()
                    && p.server() == candidate.server()
            }) {
                inherited.push(existing.clone());
                if existing.same_definition(&candidate) {
                    pools.push(existing.clone());
                    continue;
                }
                if !candidate.inherit_workers(existing) {
                    retired.push(existing.clone());
                }
                replaced.push(existing.clone());
            }
            pools.push(Arc::new(candidate));
        }
        // Preserve lazily discovered pools only while their owning site is unchanged.
        for existing in &old {
            if inherited.iter().any(|p| Arc::ptr_eq(p, existing)) {
                continue;
            }
            let discovered = !previous
                .config
                .apps
                .iter()
                .any(|a| a.name == existing.name())
                && !previous
                    .config
                    .upstreams
                    .iter()
                    .any(|a| a.name == existing.name());
            let unchanged_site = discovered
                && previous.config.sites.iter().any(|site| {
                    site.app_defaults.server == existing.server()
                        && crate::config::matches_mount(existing.mount(), &site.mount)
                        && config.sites.iter().any(|s| {
                            serde_json::to_value(s).ok() == serde_json::to_value(site).ok()
                        })
                });
            if unchanged_site
                && !pools
                    .iter()
                    .any(|p| p.server() == existing.server() && p.mount() == existing.mount())
            {
                pools.push(existing.clone());
            } else {
                retired.push(existing.clone());
            }
        }
        let state = Arc::new(AppState {
            response_headers: config.validated_response_headers()?,
            private_paths: Arc::new(crate::site::private_paths(&config, self.path.as_deref())),
            access_log: previous.access_log.clone(),
            templates_enabled: config.uses_templates(),
            auth: Auth::new(config.users.clone(), previous.secret.clone())
                .with_proxy(config.auth_proxy.clone()),
            config: Arc::new(config),
            pools: RwLock::new(pools),
            startup: previous.startup.clone(),
            log_tasks: previous.log_tasks.clone(),
            client: previous.client.clone(),
            #[cfg(unix)]
            socket_client: previous.socket_client.clone(),
            analytics: previous.analytics.clone(),
            secret: previous.secret.clone(),
            admin_token: previous.admin_token.clone(),
            slots: previous.slots.clone(),
            force_stop: previous.force_stop.clone(),
            tunnels: previous.tunnels.clone(),
            sockjs: previous.sockjs.clone(),
            control: OnceLock::new(),
        });
        if let Some(log) = &state.access_log
            && let Err(error) = log.reopen().await
        {
            tracing::error!(%error, "Access log is unavailable during reload");
        }
        let _ = state.control.set(Arc::downgrade(self));
        {
            let mut current = self.current.write().unwrap_or_else(|p| p.into_inner());
            for pool in replaced.iter().chain(&retired) {
                pool.retire();
            }
            *current = state;
            self.retired
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .extend(retired);
        }
        tracing::info!(
            "Application configuration reloaded; existing sessions are draining normally"
        );
        Ok(())
    }
}
pub(crate) fn fixed_settings(config: &Config) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(config)?;
    for key in [
        "apps",
        "sites",
        "redirects",
        "upstreams",
        "users",
        "auth_proxy",
        "apps_dir",
        "rscript",
        "response_headers",
    ] {
        value
            .as_object_mut()
            .expect("Configuration is an object")
            .remove(key);
    }
    Ok(value)
}
