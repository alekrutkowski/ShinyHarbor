use anyhow::{Context, Result, bail, ensure};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
    pub servers: Vec<VirtualServer>,
    pub admin_listen: SocketAddr,
    pub state_dir: PathBuf,
    pub socket_dir: Option<PathBuf>,
    pub rscript: PathBuf,
    pub apps_dir: Option<PathBuf>,
    pub trusted_proxies: Vec<IpNet>,
    pub public_origin: Option<String>,
    pub http_redirect_listen: Option<SocketAddr>,
    pub redirect_http: bool,
    pub allow_plaintext_public: bool,
    pub http_allow_compression: bool,
    pub http_keepalive_timeout_seconds: f64,
    pub access_log: Option<AccessLog>,
    pub shutdown_grace_seconds: u64,
    pub response_header_timeout_seconds: u64,
    pub max_inflight: usize,
    pub sockjs_heartbeat_seconds: f64,
    pub sockjs_disconnect_seconds: f64,
    pub tls: Tls,
    pub analytics: Analytics,
    pub users: BTreeMap<String, String>,
    pub auth_proxy: Option<ProxyAuth>,
    pub template_dir: Option<PathBuf>,
    pub apps: Vec<App>,
    pub sites: Vec<Site>,
    pub redirects: Vec<Redirect>,
    pub upstreams: Vec<Upstream>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:3838".parse().unwrap(),
            servers: vec![],
            admin_listen: "127.0.0.1:3839".parse().unwrap(),
            state_dir: ".harbor".into(),
            socket_dir: None,
            rscript: "Rscript".into(),
            apps_dir: None,
            trusted_proxies: vec![],
            public_origin: None,
            http_redirect_listen: None,
            redirect_http: true,
            allow_plaintext_public: false,
            http_allow_compression: true,
            http_keepalive_timeout_seconds: 45.0,
            access_log: None,
            shutdown_grace_seconds: 30,
            response_header_timeout_seconds: 120,
            max_inflight: 8192,
            sockjs_heartbeat_seconds: 25.0,
            sockjs_disconnect_seconds: 5.0,
            tls: Tls::default(),
            analytics: Analytics::default(),
            users: BTreeMap::new(),
            auth_proxy: None,
            template_dir: None,
            apps: vec![],
            sites: vec![],
            redirects: vec![],
            upstreams: vec![],
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLog {
    pub path: PathBuf,
    #[serde(default = "default_access_format")]
    pub format: String,
}
fn default_access_format() -> String {
    "default".into()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyAuth {
    pub user_header: String,
    pub groups_header: Option<String>,
    /// Transport peers, never addresses supplied in forwarding headers.
    pub trusted_peers: Vec<IpNet>,
}
impl Default for ProxyAuth {
    fn default() -> Self {
        Self {
            user_header: "x-auth-username".into(),
            groups_header: None,
            trusted_peers: vec!["127.0.0.1/32".parse().unwrap(), "::1/128".parse().unwrap()],
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Tls {
    Off {},
    Pem {
        cert: PathBuf,
        key: PathBuf,
    },
    Acme {
        domains: Vec<String>,
        email: String,
        #[serde(default)]
        production: bool,
        #[serde(default)]
        directory_url: Option<String>,
        #[serde(default)]
        ca_bundle: Option<PathBuf>,
    },
}
impl Default for Tls {
    fn default() -> Self {
        Self::Off {}
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Analytics {
    pub enabled: bool,
    pub retention_days: u32,
    pub queue_capacity: usize,
    pub country_database: Option<PathBuf>,
}
impl Default for Analytics {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
            queue_capacity: 8192,
            country_database: None,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct App {
    pub server: usize,
    pub name: String,
    pub label: Option<String>,
    pub mount: String,
    pub path: PathBuf,
    pub rscript: Option<PathBuf>,
    pub max_workers: usize,
    pub sessions_per_worker: usize,
    pub load_factor: f64,
    pub session_timeout_seconds: u64,
    pub idle_seconds: u64,
    pub startup_seconds: u64,
    pub environment: BTreeMap<String, String>,
    pub allowed_users: Vec<String>,
    pub allowed_groups: Vec<String>,
    pub bookmark_state_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub template_dir: Option<PathBuf>,
    pub log_file_mode: String,
    pub preserve_logs: bool,
    pub log_as_user: bool,
    pub sanitize_errors: bool,
    pub frame_options: FrameOptions,
    pub google_analytics_id: Option<String>,
    pub disable_protocols: Vec<String>,
    pub allow_app_override: bool,
    pub reconnect: bool,
    pub run_as: Vec<String>,
    /// OS groups allowed to deploy through user_dirs; independent of viewer ACLs.
    pub members_of: Vec<String>,
}
impl Default for App {
    fn default() -> Self {
        Self {
            server: 0,
            name: String::new(),
            label: None,
            mount: String::new(),
            path: PathBuf::new(),
            rscript: None,
            max_workers: 4,
            sessions_per_worker: 20,
            load_factor: 1.0,
            session_timeout_seconds: 0,
            idle_seconds: 60,
            startup_seconds: 60,
            environment: BTreeMap::new(),
            allowed_users: vec![],
            allowed_groups: vec![],
            bookmark_state_dir: None,
            log_dir: None,
            template_dir: None,
            log_file_mode: "0640".into(),
            preserve_logs: false,
            log_as_user: false,
            sanitize_errors: true,
            frame_options: FrameOptions::Allow,
            google_analytics_id: None,
            disable_protocols: vec![],
            allow_app_override: false,
            reconnect: true,
            run_as: vec![":PROCESS_USER:".into()],
            members_of: vec![],
        }
    }
}
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameOptions {
    #[default]
    Allow,
    Deny,
    Sameorigin,
    AllowFrom(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    #[serde(default)]
    pub user_dirs: bool,
    pub name: String,
    pub mount: String,
    pub path: PathBuf,
    #[serde(default)]
    pub directory_index: bool,
    #[serde(default)]
    pub app_defaults: App,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Redirect {
    #[serde(default)]
    pub server: usize,
    pub name: String,
    pub mount: String,
    pub target: String,
    pub status: u16,
    pub exact: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    #[serde(default)]
    pub server: usize,
    pub name: String,
    pub mount: String,
    /// Full upstream origins, not arbitrary target URLs. Paths and queries are
    /// forwarded unchanged. Place the same location tree on every replica.
    pub backends: Vec<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualServer {
    pub listen: SocketAddr,
    #[serde(default)]
    pub template_dir: Option<PathBuf>,
    #[serde(default)]
    pub server_names: Vec<String>,
}
impl Config {
    pub fn uses_templates(&self) -> bool {
        self.template_dir.is_some()
            || self.servers.iter().any(|s| s.template_dir.is_some())
            || self.apps.iter().any(|a| a.template_dir.is_some())
            || self
                .sites
                .iter()
                .any(|s| s.app_defaults.template_dir.is_some())
    }
    pub fn redirect_listener(&self) -> Option<SocketAddr> {
        if !self.redirect_http {
            return None;
        }
        self.http_redirect_listen.or_else(|| {
            matches!(self.tls, Tls::Acme { .. }).then(|| SocketAddr::new(self.listen.ip(), 80))
        })
    }
    pub fn listeners(&self) -> Vec<SocketAddr> {
        let mut listeners: Vec<_> = if self.servers.is_empty() {
            vec![self.listen]
        } else {
            self.servers.iter().map(|s| s.listen).collect()
        };
        listeners.sort();
        listeners.dedup();
        listeners
    }
    pub fn select_server(&self, listen: SocketAddr, host: &str) -> Option<usize> {
        if self.servers.is_empty() {
            return Some(0);
        }
        let host = host
            .parse::<http::uri::Authority>()
            .ok()?
            .host()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.servers
            .iter()
            .enumerate()
            .find(|(_, s)| {
                s.listen == listen
                    && s.server_names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&host))
            })
            .map(|(i, _)| i)
            .or_else(|| {
                self.servers
                    .iter()
                    .position(|s| s.listen == listen && s.server_names.is_empty())
            })
    }
    pub fn load(path: &Path) -> Result<Self> {
        let absolute = std::fs::canonicalize(path).with_context(|| {
            format!(
                "Cannot read configuration {}. Run `shinyharbor init` first.",
                path.display()
            )
        })?;
        let raw = std::fs::read_to_string(&absolute)?;
        let mut cfg: Self = if absolute.extension().is_some_and(|e| e == "conf") {
            crate::legacy::parse(&raw)
                .with_context(|| format!("Invalid Shiny Server configuration {}", path.display()))?
        } else {
            toml::from_str(&raw).context(
                "Invalid TOML configuration (use a .conf extension for Shiny Server syntax)",
            )?
        };
        let base = absolute
            .parent()
            .context("Configuration has no parent directory")?;
        cfg.state_dir = relative(base, &cfg.state_dir);
        if let Some(path) = &mut cfg.socket_dir {
            *path = relative(base, path);
        }
        if let Some(log) = &mut cfg.access_log {
            log.path = relative(base, &log.path);
        }
        if let Some(path) = &mut cfg.template_dir {
            *path = relative(base, path);
        }
        for server in &mut cfg.servers {
            if let Some(path) = &mut server.template_dir {
                *path = relative(base, path);
            }
        }
        if cfg.rscript.components().count() > 1 {
            cfg.rscript = relative(base, &cfg.rscript);
        }
        if let Some(dir) = cfg.apps_dir.take() {
            let dir = relative(base, &dir);
            cfg.discover(&dir)?;
            cfg.apps_dir = Some(dir);
        }
        for site in &mut cfg.sites {
            site.path = std::fs::canonicalize(relative(base, &site.path))
                .with_context(|| format!("Cannot open site directory {}", site.path.display()))?;
        }
        for app in cfg
            .apps
            .iter_mut()
            .chain(cfg.sites.iter_mut().map(|s| &mut s.app_defaults))
        {
            app.path = relative(base, &app.path);
            if let Some(p) = &mut app.bookmark_state_dir {
                *p = relative(base, p);
            }
            if let Some(p) = &mut app.log_dir {
                *p = relative(base, p);
            }
            app.template_dir = match app.template_dir.take() {
                Some(path) => Some(relative(base, &path)),
                None => cfg
                    .servers
                    .get(app.server)
                    .and_then(|s| s.template_dir.clone())
                    .or_else(|| cfg.template_dir.clone()),
            };
            if let Some(r) = &mut app.rscript
                && r.components().count() > 1
            {
                *r = relative(base, r);
            }
        }
        match &mut cfg.tls {
            Tls::Pem { cert, key } => {
                *cert = relative(base, cert);
                *key = relative(base, key);
            }
            Tls::Acme {
                ca_bundle: Some(path),
                ..
            } => *path = relative(base, path),
            Tls::Off {} | Tls::Acme { .. } => {}
        }
        if let Some(p) = &mut cfg.analytics.country_database {
            *p = relative(base, p);
        }
        for mount in cfg
            .apps
            .iter_mut()
            .map(|a| &mut a.mount)
            .chain(cfg.sites.iter_mut().map(|s| &mut s.mount))
            .chain(cfg.redirects.iter_mut().map(|r| &mut r.mount))
            .chain(cfg.upstreams.iter_mut().map(|u| &mut u.mount))
        {
            ensure!(valid_mount(mount), "Invalid route mount {mount:?}");
            *mount = canonical_path(mount).context("Invalid route encoding")?;
        }
        cfg.validate()?;
        let mut templates: Vec<_> = cfg
            .apps
            .iter()
            .filter_map(|a| a.template_dir.as_ref())
            .chain(
                cfg.sites
                    .iter()
                    .filter_map(|s| s.app_defaults.template_dir.as_ref()),
            )
            .chain(cfg.servers.iter().filter_map(|s| s.template_dir.as_ref()))
            .chain(cfg.template_dir.iter())
            .collect();
        templates.sort();
        templates.dedup();
        for directory in templates {
            crate::templates::validate(directory)?;
        }
        Ok(cfg)
    }
    fn discover(&mut self, root: &Path) -> Result<()> {
        let mut entries = std::fs::read_dir(root)
            .with_context(|| format!("Cannot scan {}", root.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !is_app(&p) {
                continue;
            }
            let identifier = if safe_name(&name) {
                name.clone()
            } else {
                use sha2::Digest;
                format!(
                    "app-{}",
                    hex::encode(&sha2::Sha256::digest(name.as_bytes())[..12])
                )
            };
            if self.apps.iter().any(|a| a.name == identifier) {
                continue;
            }
            self.apps.push(App {
                mount: format!("/{}", encode_segment(&name)),
                name: identifier,
                label: Some(name),
                path: p,
                ..App::default()
            });
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.socket_dir.is_none() || cfg!(unix),
            "socket_dir requires Unix domain sockets; omit it on Windows to use private loopback workers"
        );
        ensure!(
            self.http_keepalive_timeout_seconds.is_finite()
                && (0.0..=86400.0).contains(&self.http_keepalive_timeout_seconds),
            "HTTP keepalive timeout must be 0..86400 seconds (0 disables the deadline)"
        );
        if let Some(log) = &self.access_log {
            crate::access_log::validate(&log.format)?;
            ensure!(
                !log.path.as_os_str().is_empty(),
                "Access log path must not be empty"
            );
        }
        if let Some(address) = self.redirect_listener() {
            ensure!(
                !matches!(self.tls, Tls::Off {}),
                "HTTP redirect requires TLS"
            );
            ensure!(
                address.port() > 0
                    && address != self.admin_listen
                    && !self.listeners().contains(&address),
                "HTTP redirect listener must use a separate port"
            );
            ensure!(
                matches!(self.tls, Tls::Acme { .. }) || self.public_origin.is_some(),
                "PEM redirects require public_origin"
            );
        }
        ensure!(
            self.listen.port() > 0 && self.admin_listen.port() > 0,
            "Listening ports must not be zero"
        );
        ensure!(
            self.listen != self.admin_listen,
            "Application and admin listeners must differ"
        );
        for listen in self.listeners() {
            ensure!(
                listen.port() > 0 && listen != self.admin_listen,
                "Invalid public listener {listen}"
            );
            ensure!(
                listen.ip().is_loopback()
                    || !matches!(self.tls, Tls::Off {})
                    || self.allow_plaintext_public,
                "Public plaintext is disabled on {listen}"
            );
        }
        for server in &self.servers {
            for name in &server.server_names {
                ensure!(
                    !name.is_empty() && name.parse::<http::uri::Authority>().is_ok(),
                    "Invalid server_name {name:?}"
                );
            }
        }
        ensure!(
            self.admin_listen.ip().is_loopback(),
            "Admin must bind to loopback; publish it through an authenticated TLS proxy or an SSH tunnel"
        );
        ensure!(self.max_inflight > 0, "max_inflight must be positive");
        for (name, value) in [
            ("sockjs_heartbeat_seconds", self.sockjs_heartbeat_seconds),
            ("sockjs_disconnect_seconds", self.sockjs_disconnect_seconds),
        ] {
            ensure!(
                value.is_finite() && value > 0.0 && value <= 86400.0,
                "{name} must be positive and at most 86400"
            );
        }
        ensure!(
            self.response_header_timeout_seconds > 0,
            "response_header_timeout_seconds must be positive"
        );
        ensure!(
            self.analytics.retention_days > 0 && self.analytics.retention_days <= 366,
            "retention_days must be between 1 and 366"
        );
        ensure!(
            self.analytics.queue_capacity > 0,
            "queue_capacity must be positive"
        );
        if matches!(self.tls, Tls::Off {}) && !self.listen.ip().is_loopback() {
            ensure!(
                self.allow_plaintext_public,
                "Public plaintext is disabled. Configure TLS, or explicitly set allow_plaintext_public behind a TLS proxy."
            );
        }
        if let Some(origin) = &self.public_origin {
            let u = url::Url::parse(origin)?;
            ensure!(
                u.scheme() == "https"
                    && u.host_str().is_some()
                    && u.path() == "/"
                    && u.query().is_none()
                    && u.fragment().is_none()
                    && u.username().is_empty()
                    && u.password().is_none(),
                "public_origin must be an HTTPS origin without credentials, a path, query or fragment"
            );
        }
        match &self.tls {
            Tls::Acme {
                domains,
                email,
                directory_url,
                ca_bundle,
                ..
            } => {
                if let Some(directory) = directory_url {
                    let url = url::Url::parse(directory)?;
                    ensure!(
                        url.scheme() == "https"
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.fragment().is_none(),
                        "ACME directory_url must be an HTTPS URL without credentials or fragment"
                    );
                }
                if let Some(ca) = ca_bundle {
                    ensure!(
                        directory_url.is_some() && ca.is_file(),
                        "ACME ca_bundle requires a custom directory_url and an existing PEM file"
                    );
                }
                ensure!(
                    self.listeners().len() == 1,
                    "ACME requires one HTTPS listener; virtual hosts may share it"
                );
                ensure!(!domains.is_empty(), "ACME needs at least one domain");
                ensure!(
                    email.contains('@') && !email.chars().any(char::is_whitespace),
                    "Invalid ACME contact email"
                );
                ensure!(
                    self.listen.port() == 443 || directory_url.is_some(),
                    "TLS-ALPN-01 requires public port 443; listen on 443 (a port-forwarded deployment needs explicit review)"
                );
                for d in domains {
                    ensure!(
                        valid_domain(d) || (directory_url.is_some() && d == "localhost"),
                        "Invalid ACME DNS name {d:?}; wildcards and IP addresses are not supported"
                    );
                }
            }
            Tls::Pem { cert, key } => ensure!(
                cert.is_file() && key.is_file(),
                "Certificate and key files must exist"
            ),
            Tls::Off {} => {}
        }
        let mut names = HashSet::new();
        let mut mounts = HashSet::new();
        for (name, mount, server) in self
            .apps
            .iter()
            .map(|a| (&a.name, &a.mount, a.server))
            .chain(self.upstreams.iter().map(|a| (&a.name, &a.mount, a.server)))
            .chain(
                self.sites
                    .iter()
                    .map(|s| (&s.name, &s.mount, s.app_defaults.server)),
            )
            .chain(self.redirects.iter().map(|r| (&r.name, &r.mount, r.server)))
        {
            ensure!(
                server < self.servers.len().max(1),
                "Route {name} references an unknown server"
            );
            ensure!(
                safe_name(name),
                "Route name {name:?} must contain only letters, digits, '-', '_' or '.'"
            );
            ensure!(
                valid_mount(mount),
                "Invalid route mount {mount:?}; use / or /name without a trailing slash"
            );
            ensure!(names.insert(name), "Duplicate route name: {name}");
            ensure!(
                mounts.insert((server, mount)),
                "Duplicate route mount: {mount}"
            );
        }
        for app in self
            .apps
            .iter()
            .chain(self.sites.iter().map(|s| &s.app_defaults))
        {
            ensure!(
                app.path.exists(),
                "Application {} does not exist: {}",
                app.name,
                app.path.display()
            );
            ensure!(
                app.max_workers > 0 && app.sessions_per_worker > 0 && app.startup_seconds > 0,
                "Application {} has a zero worker/session/startup limit",
                app.name
            );
            ensure!(
                app.load_factor.is_finite() && app.load_factor > 0.0 && app.load_factor <= 1.0,
                "Application {} load_factor must be in (0, 1]",
                app.name
            );
            log_mode(&app.log_file_mode)?;
            if let FrameOptions::AllowFrom(origin) = &app.frame_options {
                let url = url::Url::parse(origin)
                    .context("frame_options allow-from needs an absolute HTTP(S) URL")?;
                ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && origin.parse::<http::HeaderValue>().is_ok(),
                    "Invalid frame_options allow-from URL"
                );
            }
            if let Some(id) = &app.google_analytics_id {
                ensure!(
                    id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                    "Invalid Google Analytics tag ID"
                );
            }
            ensure!(
                app.session_timeout_seconds <= 366 * 86400,
                "Session timeout exceeds one year"
            );
            ensure!(
                !app.environment.keys().any(|k| k.starts_with("HARBOR_")),
                "HARBOR_* environment names are reserved"
            );
            for protocol in &app.disable_protocols {
                ensure!(
                    crate::sockjs::valid_protocol(protocol),
                    "Unknown SockJS protocol {protocol:?}"
                );
            }
        }
        for site in &self.sites {
            ensure!(
                !site.user_dirs || cfg!(unix) || site.app_defaults.members_of.is_empty(),
                "members_of requires Unix OS group lookup"
            );
            ensure!(
                site.path.is_dir(),
                "Site {} must point to a directory",
                site.name
            );
        }
        for redirect in &self.redirects {
            ensure!(
                [301, 302, 303, 307, 308].contains(&redirect.status),
                "Invalid redirect status"
            );
            ensure!(
                http::HeaderValue::from_str(&redirect.target).is_ok(),
                "Invalid redirect target"
            );
            ensure!(
                redirect.target.starts_with('/') && !redirect.target.starts_with("//")
                    || url::Url::parse(&redirect.target)
                        .is_ok_and(|u| matches!(u.scheme(), "http" | "https")),
                "Redirect target must be an absolute path or HTTP(S) URL"
            );
        }
        for up in &self.upstreams {
            ensure!(
                !up.backends.is_empty(),
                "Upstream {} has no backends",
                up.name
            );
            let mut urls = HashSet::new();
            for backend in &up.backends {
                let u = url::Url::parse(backend).context("Invalid upstream URL")?;
                // Upstreams must be on a trusted local/private network. TLS to a
                // remote upstream is deliberately not silently downgraded.
                ensure!(
                    u.scheme() == "http"
                        && u.host_str().is_some()
                        && u.path() == "/"
                        && u.query().is_none()
                        && u.fragment().is_none()
                        && u.username().is_empty()
                        && u.password().is_none(),
                    "Backend must be an http:// origin, without credentials, path, query or fragment"
                );
                ensure!(urls.insert(backend), "Duplicate backend in {}", up.name);
            }
        }
        if let Some(proxy) = &self.auth_proxy {
            ensure!(
                !proxy.trusted_peers.is_empty(),
                "auth_proxy requires trusted_peers"
            );
            for name in std::iter::once(&proxy.user_header).chain(proxy.groups_header.iter()) {
                let header = http::header::HeaderName::from_bytes(name.as_bytes())
                    .context("Invalid authentication proxy header")?;
                ensure!(
                    !matches!(
                        header.as_str(),
                        "host"
                            | "cookie"
                            | "authorization"
                            | "connection"
                            | "content-length"
                            | "transfer-encoding"
                            | "upgrade"
                            | "shiny-shared-secret"
                            | "shiny-server-credentials"
                    ) && !header.as_str().starts_with("sec-"),
                    "Reserved authentication proxy header {name}"
                );
            }
            ensure!(
                proxy
                    .groups_header
                    .as_ref()
                    .is_none_or(|g| !g.eq_ignore_ascii_case(&proxy.user_header)),
                "Authentication user and groups headers must differ"
            );
        }
        for (allowed, groups) in self
            .apps
            .iter()
            .map(|a| (&a.allowed_users, &a.allowed_groups))
            .chain(
                self.upstreams
                    .iter()
                    .map(|a| (&a.allowed_users, &a.allowed_groups)),
            )
            .chain(self.sites.iter().map(|s| {
                (
                    &s.app_defaults.allowed_users,
                    &s.app_defaults.allowed_groups,
                )
            }))
        {
            for user in allowed {
                ensure!(crate::auth::valid_identity(user), "Invalid allowed user");
                ensure!(
                    self.auth_proxy.is_some()
                        || matches!(user.as_str(), "*" | "**")
                        || self.users.contains_key(user),
                    "Unknown allowed user {user:?}"
                );
            }
            ensure!(
                groups.is_empty()
                    || self
                        .auth_proxy
                        .as_ref()
                        .is_some_and(|p| p.groups_header.is_some()),
                "allowed_groups requires auth_proxy.groups_header"
            );
            ensure!(
                groups.iter().all(|g| crate::auth::valid_identity(g)),
                "Invalid allowed group"
            );
        }
        for (user, hash) in &self.users {
            ensure!(safe_name(user), "Invalid username {user:?}");
            argon2::PasswordHash::new(hash)
                .map_err(|e| anyhow::anyhow!("Invalid hash for {user}: {e}"))?;
        }
        if self.apps.is_empty()
            && self.upstreams.is_empty()
            && self.sites.is_empty()
            && self.redirects.is_empty()
        {
            bail!("No applications, sites, redirects or upstream routes configured");
        }
        Ok(())
    }
}
fn relative(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_owned()
    } else {
        base.join(p)
    }
}
pub fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub fn log_mode(mode: &str) -> Result<u32> {
    ensure!(
        (3..=4).contains(&mode.len()) && mode.bytes().all(|b| (b'0'..=b'7').contains(&b)),
        "log_file_mode must be an octal permission such as 0640"
    );
    let mode = u32::from_str_radix(mode, 8)?;
    ensure!(
        mode <= 0o777,
        "log_file_mode cannot set special permission bits"
    );
    Ok(mode)
}
pub fn valid_mount(m: &str) -> bool {
    m == "/"
        || (m.starts_with('/')
            && !m.ends_with('/')
            && !m.contains(['?', '#', '\\'])
            && canonical_path(m).is_some()
            && m[1..].split('/').all(|part| {
                percent_encoding::percent_decode_str(part)
                    .decode_utf8()
                    .is_ok_and(|p| {
                        !p.is_empty()
                            && p.len() <= 255
                            && p != "."
                            && p != ".."
                            && !p.contains(['/', '\\'])
                            && !p.chars().any(char::is_control)
                    })
            }))
}
pub fn encode_segment(segment: &str) -> String {
    const ESCAPE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(segment, ESCAPE).to_string()
}
/// Normalize URL spelling for routing; never rewrite the forwarded request URI.
pub fn canonical_path(path: &str) -> Option<String> {
    if path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
    {
        return Some(path.to_owned());
    }
    let bytes = path.as_bytes();
    if bytes.iter().enumerate().any(|(i, b)| {
        *b == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
    }) {
        return None;
    }
    path.split('/')
        .map(|p| {
            percent_encoding::percent_decode_str(p)
                .decode_utf8()
                .ok()
                .map(|p| encode_segment(&p))
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
/// Strip a matched mount by path segments, preserving the suffix's original bytes.
pub fn relative_path<'a>(path: &'a str, mount: &str) -> Option<&'a str> {
    if mount == "/" {
        return Some(path);
    }
    if matches_mount(path, mount) {
        return path.strip_prefix(mount);
    }
    let count = mount.trim_start_matches('/').split('/').count();
    let index = path
        .match_indices('/')
        .nth(count)
        .map_or(path.len(), |(index, _)| index);
    if canonical_path(&path[..index])? != mount {
        return None;
    }
    Some(&path[index..])
}
pub fn matches_mount(path: &str, mount: &str) -> bool {
    mount == "/" || path == mount || path.strip_prefix(mount).is_some_and(|s| s.starts_with('/'))
}
fn valid_domain(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.parse::<std::net::IpAddr>().is_err()
        && d.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
pub(crate) fn is_app(p: &Path) -> bool {
    p.is_dir()
        && ["app.R", "app.r", "server.R", "server.r"]
            .iter()
            .any(|f| p.join(f).is_file())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_boundaries() {
        assert!(matches_mount("/app/asset", "/app"));
        assert!(!matches_mount("/application", "/app"));
        assert!(matches_mount("/any", "/"));
        assert_eq!(
            canonical_path("/Za%c5%bc%C3%B3%C5%82%C4%87/"),
            Some("/Za%C5%BC%C3%B3%C5%82%C4%87/".into())
        );
        assert_eq!(
            relative_path("/caf%c3%a9/session/%ff%2f?", "/caf%C3%A9"),
            Some("/session/%ff%2f?")
        );
        assert_eq!(relative_path("/app%2fother/file", "/app"), None);
        assert!(valid_mount("/Raport 日本語"));
    }
    #[test]
    fn unsafe_mounts() {
        for m in ["/a/..", "/a/", "a", "/a%2fb", "/a?b", "//a", "/a\\b"] {
            assert!(!valid_mount(m));
        }
        assert!(valid_mount("/reports/sales"));
    }
    #[test]
    fn unknown_configuration_is_an_error() {
        assert!(toml::from_str::<Config>("max_worker = 3").is_err());
        assert!(toml::from_str::<Config>("[tls]\nmode='off'\nignore=true").is_err());
    }
    #[test]
    fn acme_domains() {
        assert!(valid_domain("shiny.example.org"));
        for d in [
            "*.example.org",
            "127.0.0.1",
            "localhost",
            "-a.org",
            "a..org",
            "a.org/path",
        ] {
            assert!(!valid_domain(d));
        }
    }
}
