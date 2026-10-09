//! Parser for Shiny Server's directive syntax. Unsupported runtime directives
//! fail closed, with source positions; parsing a directive does not claim parity.
use crate::config::{App, Config, FrameOptions, Redirect, Site, VirtualServer, valid_mount};
use anyhow::{Result, bail, ensure};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Word(String),
    Open,
    Close,
    End,
}
#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    line: usize,
    column: usize,
}
#[derive(Debug)]
struct Directive {
    name: String,
    args: Vec<String>,
    children: Option<Vec<Directive>>,
    line: usize,
    column: usize,
}
impl Directive {
    fn error(&self, message: &str) -> anyhow::Error {
        anyhow::anyhow!("{}:{}: {}: {}", self.line, self.column, self.name, message)
    }
    fn arity(&self, min: usize, max: usize) -> Result<()> {
        if !(min..=max).contains(&self.args.len()) {
            return Err(self.error(&format!(
                "expected {min}..={max} arguments, got {}",
                self.args.len()
            )));
        }
        Ok(())
    }
    fn one(&self) -> Result<&str> {
        self.arity(1, 1)?;
        Ok(&self.args[0])
    }
    fn number(&self) -> Result<u64> {
        self.one()?
            .parse()
            .map_err(|_| self.error("expected a nonnegative integer"))
    }
    fn boolean(&self, default: Option<bool>) -> Result<bool> {
        self.arity(usize::from(default.is_none()), 1)?;
        match self.args.first() {
            None => Ok(default.unwrap_or(false)),
            Some(value) => boolean(value)
                .ok_or_else(|| self.error("expected on, off, true, false, yes, no, 1 or 0")),
        }
    }
}
fn boolean(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn lex(input: &str) -> Result<Vec<Token>> {
    ensure!(input.len() <= 1024 * 1024, "Configuration exceeds 1 MiB");
    let mut chars = input.trim_start_matches('\u{feff}').chars().peekable();
    let (mut line, mut column) = (1, 1);
    let mut tokens = Vec::new();
    while let Some(c) = chars.peek().copied() {
        if c.is_whitespace() {
            chars.next();
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            continue;
        }
        if c == '#' {
            while chars.peek().is_some_and(|c| *c != '\n') {
                chars.next();
                column += 1;
            }
            continue;
        }
        let (start_line, start_column) = (line, column);
        let kind = match c {
            '{' | '}' | ';' => {
                chars.next();
                column += 1;
                match c {
                    '{' => Kind::Open,
                    '}' => Kind::Close,
                    _ => Kind::End,
                }
            }
            _ => {
                let mut word = String::new();
                let mut quote = None;
                while let Some(c) = chars.peek().copied() {
                    if quote.is_none() && (c.is_whitespace() || "{};#".contains(c)) {
                        break;
                    }
                    chars.next();
                    column += 1;
                    if c == '\n' {
                        line += 1;
                        column = 1;
                    }
                    match (quote, c) {
                        (None, '\'' | '"') => quote = Some(c),
                        (Some(q), c) if c == q => quote = None,
                        (_, '\\') => {
                            // Preserve Windows path separators. Escape syntax only
                            // consumes delimiters, quotes, whitespace and backslashes.
                            if let Some(next) = chars.peek().copied() {
                                if "\\\"'{};#".contains(next) || next.is_whitespace() {
                                    chars.next();
                                    column += 1;
                                    if next == '\n' {
                                        line += 1;
                                        column = 1;
                                    }
                                    word.push(next);
                                } else {
                                    word.push(c);
                                }
                            } else {
                                word.push(c);
                            }
                        }
                        _ => word.push(c),
                    }
                }
                ensure!(
                    quote.is_none(),
                    "{start_line}:{start_column}: unterminated quoted string"
                );
                Kind::Word(word)
            }
        };
        tokens.push(Token {
            kind,
            line: start_line,
            column: start_column,
        });
    }
    Ok(tokens)
}
fn block(tokens: &[Token], pos: &mut usize, depth: usize) -> Result<Vec<Directive>> {
    ensure!(depth <= 64, "Configuration nesting exceeds 64 levels");
    let mut out = Vec::new();
    while let Some(token) = tokens.get(*pos) {
        if token.kind == Kind::Close {
            ensure!(
                depth > 0,
                "{}:{}: unexpected closing brace",
                token.line,
                token.column
            );
            *pos += 1;
            return Ok(out);
        }
        let Kind::Word(name) = &token.kind else {
            bail!("{}:{}: expected directive name", token.line, token.column)
        };
        let mut node = Directive {
            name: name.clone(),
            args: vec![],
            children: None,
            line: token.line,
            column: token.column,
        };
        *pos += 1;
        loop {
            let next = tokens
                .get(*pos)
                .ok_or_else(|| node.error("missing semicolon or opening brace"))?;
            *pos += 1;
            match &next.kind {
                Kind::Word(value) => node.args.push(value.clone()),
                Kind::End => break,
                Kind::Open => {
                    node.children = Some(block(tokens, pos, depth + 1)?);
                    break;
                }
                Kind::Close => return Err(node.error("missing semicolon before closing brace")),
            }
        }
        out.push(node);
    }
    ensure!(depth == 0, "Unclosed configuration block");
    Ok(out)
}

#[derive(Clone)]
struct Settings {
    app: App,
    index: bool,
    hosting: Option<(bool, PathBuf)>,
    user_dirs: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            app: App {
                max_workers: 1,
                sessions_per_worker: 100,
                idle_seconds: 5,
                login_shell: cfg!(unix),
                ..App::default()
            },
            index: false,
            hosting: None,
            user_dirs: false,
        }
    }
}

fn settings(nodes: &[Directive], parent: &Settings, scope: &str) -> Result<Settings> {
    let mut result = parent.clone();
    let mut seen = HashSet::new();
    let mut hostings = 0;
    for d in nodes {
        if matches!(d.name.as_str(), "server" | "location") {
            continue;
        }
        ensure!(
            seen.insert(&d.name),
            "{}:{}: duplicate {} directive",
            d.line,
            d.column,
            d.name
        );
        if d.children.is_some() {
            return Err(d.error("this directive cannot contain a block"));
        }
        match d.name.as_str() {
            "listen" if scope == "server" => { d.arity(1, 2)?; }
            "server_name" if scope == "server" => {}
            "ssl" if scope == "server" => { d.arity(2, 2)?; }
            "allow_app_override" if scope == "root" => result.app.allow_app_override = d.boolean(Some(true))?,
            "auth_proxy" if scope == "root" => { d.arity(0, 2)?; }
            "http_allow_compression" if scope == "root" => { d.boolean(Some(true))?; }
            "access_log" if scope == "root" => { d.arity(1, 2)?; }
            "socket_dir" if scope == "root" => { d.arity(1, 1)?; }
            "http_keepalive_timeout" if scope == "root" => { d.arity(1, 1)?; }
            "required_user" => { d.arity(0, 256)?; result.app.allowed_users = d.args.clone(); }
            "required_group" => { d.arity(0, 256)?; result.app.allowed_groups = d.args.clone(); }
            "members_of" if scope != "application" => { d.arity(0, 256)?; result.app.members_of = d.args.clone(); }
            "r_path" => result.app.rscript = Some(d.one()?.into()),
            "exec_supervisor" => {
                result.app.supervisor = shell_words::split(d.one()?)
                    .map_err(|_| d.error("exec_supervisor contains unclosed quotes"))?;
            }
            "sockjs_heartbeat_delay" | "sockjs_disconnect_delay" if scope == "root" => {
                let value: f64 = d.one()?.parse().map_err(|_| d.error("expected positive seconds"))?;
                ensure!(value.is_finite() && value > 0.0 && value <= 86400.0, "Invalid SockJS timeout");
            }
            "disable_websockets" => {
                let disabled = d.boolean(Some(true))?;
                result.app.disable_protocols.retain(|p| p != "websocket");
                if disabled { result.app.disable_protocols.push("websocket".into()); }
            }
            "disable_protocols" => {
                for protocol in &d.args { if !crate::sockjs::valid_protocol(protocol) { return Err(d.error("unknown SockJS protocol")); } }
                result.app.disable_protocols = d.args.clone();
            }
            "app_dir" | "site_dir" if scope == "location" => {
                result.user_dirs = false;
                result.hosting = Some((d.name == "site_dir", PathBuf::from(d.one()?))); hostings += 1;
            }
            "user_dirs" | "user_apps" if scope == "location" => {
                result.user_dirs = d.boolean(Some(true))?;
                if result.user_dirs {
                    result.hosting = Some((true, PathBuf::from("/"))); hostings += 1;
                    if d.name == "user_apps" { result.app.run_as = vec![":HOME_USER:".into()]; }
                }
            }
            "redirect" if scope == "location" => { d.arity(1, 3)?; hostings += 1; }
            "run_as" => {
                d.arity(1, 64)?;
                for user in &d.args {
                    if user.starts_with(':') && !matches!(user.as_str(), ":PROCESS_USER:" | ":HOME_USER:") {
                        return Err(d.error("unknown run_as placeholder"));
                    }
                }
                result.app.run_as = d.args.clone();
            }
            "app_init_timeout" => {
                result.app.startup_seconds = d.number()?;
                if result.app.startup_seconds == 0 { return Err(d.error("startup timeout must be positive")); }
            }
            "app_idle_timeout" => result.app.idle_seconds = d.number()?,
            "app_session_timeout" => {
                result.app.session_timeout_seconds = d.number()?;
                ensure!(result.app.session_timeout_seconds <= 366 * 86400, "Session timeout exceeds one year");
            }
            "utilization_scheduler" => {
                d.arity(3, 3)?;
                result.app.sessions_per_worker = d.args[0].parse().map_err(|_| d.error("invalid request limit"))?;
                result.app.load_factor = d.args[1].parse().map_err(|_| d.error("invalid load factor"))?;
                result.app.max_workers = d.args[2].parse().map_err(|_| d.error("invalid worker limit"))?;
                ensure!(result.app.sessions_per_worker > 0 && result.app.max_workers > 0 && result.app.load_factor.is_finite() && result.app.load_factor > 0.0 && result.app.load_factor <= 1.0, "Invalid utilization scheduler parameters");
            }
            "simple_scheduler" => {
                d.arity(0, 1)?;
                result.app.max_workers = 1;
                result.app.load_factor = 1.0;
                result.app.sessions_per_worker = if d.args.is_empty() { 100 } else {
                    usize::try_from(d.number()?).map_err(|_| d.error("session limit is too large"))?
                };
                if result.app.sessions_per_worker == 0 { return Err(d.error("session limit must be positive")); }
            }
            "directory_index" => result.index = d.boolean(None)?,
            "sanitize_errors" => result.app.sanitize_errors = d.boolean(Some(true))?,
            "reconnect" => result.app.reconnect = d.boolean(Some(true))?,
            "bookmark_state_dir" => result.app.bookmark_state_dir = Some(d.one()?.into()),
            "log_dir" => result.app.log_dir = Some(d.one()?.into()),
            "template_dir" if scope != "application" => result.app.template_dir = Some(d.one()?.into()),
            "google_analytics_id" if scope != "application" => result.app.google_analytics_id = Some(d.one()?.into()),
            "log_as_user" if scope != "application" => result.app.log_as_user = d.boolean(Some(false))?,
            "log_file_mode" => {
                d.arity(0, 1)?;
                result.app.log_file_mode = d.args.first().cloned().unwrap_or_else(|| "0640".into());
                crate::config::log_mode(&result.app.log_file_mode)?;
            }
            "preserve_logs" if matches!(scope, "root" | "application") => result.app.preserve_logs = d.boolean(None)?,
            "frame_options" => {
                d.arity(1, 2)?;
                result.app.frame_options = match d.args[0].to_ascii_lowercase().as_str() {
                    "allow-from" => { d.arity(2, 2)?; FrameOptions::AllowFrom(d.args[1].clone()) },
                    other => { d.arity(1, 1)?; match other {
                        "allow" => FrameOptions::Allow, "deny" => FrameOptions::Deny, "sameorigin" => FrameOptions::Sameorigin,
                        _ => return Err(d.error("supported frame options: allow, deny, sameorigin, allow-from URL")),
                    } }
                };
            },
            // Upstream explicitly documents this directive as ignored.
            "application" if scope == "location" => { d.arity(1, 1)?; }
            _ => return Err(d.error("unsupported directive or invalid scope; it has not been ignored. See docs/COMPATIBILITY.md")),
        }
    }
    ensure!(
        hostings <= 1,
        "A location may have only one hosting directive (app_dir, site_dir or redirect)"
    );
    Ok(result)
}

/// Only upstream's explicitly overridable settings are accepted in app files.
pub fn app_settings(app: &App) -> Result<App> {
    if !app.allow_app_override {
        return Ok(app.clone());
    }
    let directory = if app.path.is_dir() {
        app.path.as_path()
    } else {
        app.path.parent().unwrap_or(&app.path)
    };
    let path = directory.join(".shiny_app.conf");
    let input = match std::fs::read_to_string(&path) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(app.clone()),
        Err(error) => return Err(error.into()),
    };
    let tokens = lex(&input)?;
    let nodes = block(&tokens, &mut 0, 0)?;
    for d in &nodes {
        ensure!(
            matches!(
                d.name.as_str(),
                "app_init_timeout"
                    | "app_idle_timeout"
                    | "app_session_timeout"
                    | "simple_scheduler"
                    | "utilization_scheduler"
                    | "bookmark_state_dir"
                    | "disable_protocols"
                    | "frame_options"
                    | "r_path"
                    | "sanitize_errors"
                    | "reconnect"
                    | "required_user"
                    | "required_group"
                    | "log_file_mode"
                    | "preserve_logs"
            ),
            "{}: {} cannot be overridden",
            path.display(),
            d.name
        );
    }
    let mut result = settings(
        &nodes,
        &Settings {
            app: app.clone(),
            index: false,
            hosting: None,
            user_dirs: false,
        },
        "application",
    )?
    .app;
    if let Some(path) = &mut result.bookmark_state_dir
        && path.is_relative()
    {
        *path = directory.join(&path);
    }
    if let Some(path) = &mut result.rscript
        && path.is_relative()
        && path.components().count() > 1
    {
        *path = directory.join(&path);
    }
    Ok(result)
}

pub fn parse(input: &str) -> Result<Config> {
    let tokens = lex(input)?;
    let mut pos = 0;
    let nodes = block(&tokens, &mut pos, 0)?;
    let root = settings(&nodes, &Settings::default(), "root")?;
    let mut cfg = Config {
        template_dir: root.app.template_dir.clone(),
        ..Config::default()
    };
    for d in &nodes {
        match d.name.as_str() {
            "http_allow_compression" => cfg.http_allow_compression = d.boolean(Some(true))?,
            "socket_dir" => cfg.socket_dir = Some(d.one()?.into()),
            "http_keepalive_timeout" => cfg.http_keepalive_timeout_seconds = d.one()?.parse()?,
            "access_log" => {
                cfg.access_log = Some(crate::config::AccessLog {
                    path: d.args[0].clone().into(),
                    format: d.args.get(1).cloned().unwrap_or_else(|| "default".into()),
                })
            }
            "auth_proxy" => {
                let mut proxy = crate::config::ProxyAuth::default();
                if let Some(name) = d.args.first() {
                    proxy.user_header = name.to_ascii_lowercase();
                }
                proxy.groups_header = d.args.get(1).map(|s| s.to_ascii_lowercase());
                cfg.auth_proxy = Some(proxy);
            }
            "sockjs_heartbeat_delay" => cfg.sockjs_heartbeat_seconds = d.one()?.parse()?,
            "sockjs_disconnect_delay" => cfg.sockjs_disconnect_seconds = d.one()?.parse()?,
            _ => {}
        }
    }
    let servers: Vec<_> = nodes.iter().filter(|d| d.name == "server").collect();
    ensure!(!servers.is_empty(), "At least one server block is required");
    ensure!(
        !nodes.iter().any(|d| d.name == "location"),
        "location must be inside server or location"
    );
    for (index, server) in servers.iter().enumerate() {
        server.arity(0, 0)?;
        let children = server
            .children
            .as_ref()
            .ok_or_else(|| server.error("expected a block"))?;
        ensure!(
            !children.iter().any(|d| d.name == "server"),
            "server cannot be nested"
        );
        let mut inherited = settings(children, &root, "server")?;
        inherited.app.server = index;
        let listen = children
            .iter()
            .find(|d| d.name == "listen")
            .ok_or_else(|| server.error("missing listen directive"))?;
        let port: u16 = listen.args[0]
            .parse()
            .map_err(|_| listen.error("invalid port"))?;
        let host = listen.args.get(1).map(String::as_str).unwrap_or("*");
        let host: IpAddr = if host == "*" {
            "0.0.0.0".parse().unwrap()
        } else {
            host.parse()
                .map_err(|_| listen.error("listen host must be an IP address or *"))?
        };
        let listen = SocketAddr::new(host, port);
        if index == 0 {
            cfg.listen = listen;
        }
        let server_names = children
            .iter()
            .find(|d| d.name == "server_name")
            .map(|d| d.args.clone())
            .unwrap_or_default();
        let tls = match children.iter().find(|d| d.name == "ssl") {
            Some(d) => crate::config::Tls::Pem {
                key: d.args[0].clone().into(),
                cert: d.args[1].clone().into(),
            },
            None => crate::config::Tls::default(),
        };
        if index == 0 {
            cfg.tls = tls.clone();
        }
        cfg.servers.push(VirtualServer {
            listen,
            tls: Some(tls),
            server_names,
            template_dir: inherited.app.template_dir.clone(),
        });
        // A legacy listen directive explicitly selects the public HTTP binding.
        cfg.allow_plaintext_public = true;
        for d in children.iter().filter(|d| d.name == "location") {
            location(d, "", &inherited, &mut cfg)?;
        }
    }
    Ok(cfg)
}

fn location(d: &Directive, prefix: &str, parent: &Settings, cfg: &mut Config) -> Result<()> {
    let local_path = d.one()?;
    let normalized = format!("/{}", local_path.trim_matches('/'));
    let local = normalized.trim_end_matches('/');
    let mount = format!("{prefix}{local}");
    let mount = if mount.is_empty() {
        "/".to_owned()
    } else {
        mount
    };
    ensure!(
        valid_mount(&mount),
        "{}:{}: invalid location path",
        d.line,
        d.column
    );
    let nodes = d
        .children
        .as_ref()
        .ok_or_else(|| d.error("location requires a block"))?;
    ensure!(
        !nodes.iter().any(|d| d.name == "server"),
        "server cannot be nested"
    );
    let mut inherited = parent.clone();
    if matches!(inherited.hosting, Some((false, _))) {
        inherited.hosting = None; // app_dir cannot be inherited by a child location.
    }
    if !inherited.user_dirs
        && let Some((true, path)) = &mut inherited.hosting
    {
        *path = path.join(local.trim_start_matches('/'));
    }
    let current = settings(nodes, &inherited, "location")?;
    let name = format!(
        "location-{}",
        hex::encode(
            &sha2::Sha256::digest(format!("{}:{mount}", parent.app.server).as_bytes())[..12]
        )
    );
    use sha2::Digest;
    if let Some(redir) = nodes.iter().find(|n| n.name == "redirect") {
        if nodes.iter().any(|n| n.name == "location") {
            return Err(redir.error("nested locations below redirects are not yet supported"));
        }
        let status = redir
            .args
            .get(1)
            .map(|s| s.parse::<u16>())
            .transpose()
            .map_err(|_| redir.error("invalid redirect status"))?
            .unwrap_or(302);
        let exact = redir
            .args
            .get(2)
            .map(|s| boolean(s).ok_or_else(|| redir.error("invalid exact boolean")))
            .transpose()?
            .unwrap_or(true);
        cfg.redirects.push(Redirect {
            server: parent.app.server,
            name,
            mount: mount.clone(),
            target: redir.args[0].clone(),
            status,
            exact,
        });
    } else if let Some((site, path)) = &current.hosting {
        let app = App {
            name: name.clone(),
            label: Some(mount.clone()),
            mount: mount.clone(),
            path: path.clone(),
            ..current.app.clone()
        };
        if *site {
            cfg.sites.push(Site {
                user_dirs: current.user_dirs,
                name,
                mount: mount.clone(),
                path: path.clone(),
                directory_index: current.index,
                app_defaults: app,
            });
        } else {
            cfg.apps.push(app);
        }
    } else if !nodes.iter().any(|d| d.name == "location") {
        return Err(d.error("location has no hosting directive to inherit"));
    }
    for child in nodes.iter().filter(|d| d.name == "location") {
        location(child, mount.trim_end_matches('/'), &current, cfg)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_listeners_and_virtual_hosts_do_not_share_routes() {
        let c = parse("server { listen 3838; server_name a.example; location / { app_dir a; } } server { listen 3838; server_name b.example; location / { app_dir b; } } server { listen 8080; location / { app_dir c; } }").unwrap();
        assert_eq!(c.listeners().len(), 2);
        assert_eq!(c.select_server(c.listen, "B.EXAMPLE:3838"), Some(1));
        assert_eq!(c.select_server(c.listen, "unknown.example"), None);
        assert_eq!(c.apps[0].server, 0);
        assert_eq!(c.apps[1].server, 1);
        assert_ne!(c.apps[0].name, c.apps[1].name);
    }
    #[test]
    fn local_overrides_are_opt_in_and_cannot_change_hosting_or_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = App {
            path: temp.path().to_owned(),
            ..App::default()
        };
        let path = temp.path().join(".shiny_app.conf");
        std::fs::write(
            &path,
            "app_idle_timeout 123; simple_scheduler 42; sanitize_errors off;",
        )
        .unwrap();
        assert_eq!(app_settings(&app).unwrap().idle_seconds, app.idle_seconds);
        app.allow_app_override = true;
        let effective = app_settings(&app).unwrap();
        assert_eq!(effective.idle_seconds, 123);
        assert_eq!(effective.sessions_per_worker, 42);
        assert!(!effective.sanitize_errors);
        std::fs::write(path, "run_as root;").unwrap();
        assert!(app_settings(&app).is_err());
    }
    #[test]
    fn nested_inheritance_and_order() {
        let c = parse(
            r#"
          run_as :PROCESS_USER:;
          server {
            location /reports {
              site_dir "C:/Shiny apps";
              location /finance { app_idle_timeout 30; }
              directory_index on;
            }
            listen 3838 127.0.0.1;
            app_idle_timeout 12;
            simple_scheduler 150;
          }
        "#,
        )
        .unwrap();
        assert_eq!(c.sites[0].app_defaults.idle_seconds, 12);
        assert_eq!(c.sites[1].app_defaults.idle_seconds, 30);
        assert_eq!(c.sites[1].app_defaults.sessions_per_worker, 150);
        assert_eq!(c.sites[1].path, PathBuf::from("C:/Shiny apps/finance"));
        assert_eq!(c.sites[1].mount, "/reports/finance");
        assert!(c.sites[1].directory_index);
    }
    #[test]
    fn comments_quotes_and_windows_paths() {
        let c = parse("# comment\nserver { listen 3838; location / { app_dir 'C:\\apps\\my app'; log_dir \"logs #1\"; } }").unwrap();
        assert_eq!(c.apps[0].path, PathBuf::from("C:\\apps\\my app"));
        assert_eq!(c.apps[0].log_dir, Some("logs #1".into()));
    }
    #[test]
    fn invalid_and_unsupported_are_not_silently_ignored() {
        for source in [
            "server { listen 3838; location / { app_dir x } }",
            "server { listen 3838; listen 8080; }",
            "server { listen 3838; location / { app_dir x; site_dir y; } }",
            "server { listen 3838; location / { app_dir 'unterminated; } }",
            "listen 3838; server {}",
            "server { listen 3838; location / {}",
            "}",
        ] {
            assert!(parse(source).is_err(), "accepted {source}");
        }
        let err = parse("server {\n listen 3838;\n unsafe_setting on;\n}")
            .unwrap_err()
            .to_string();
        assert!(err.contains("3:2: unsafe_setting"), "{err}");
    }
}
