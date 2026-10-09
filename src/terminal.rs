//! A read-only live terminal view; configuration edits belong in the admin UI.
use anyhow::{Context, Result, ensure};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap},
};
use shinyharbor::config::{Config, Tls};
use std::{
    io::IsTerminal,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Page {
    Apps,
    Dashboard,
    Admin,
}

pub fn page_url(config: &Config, page: Page) -> String {
    match page {
        Page::Admin => format!("http://{}/admin", config.admin_listen),
        Page::Dashboard => format!("http://{}/", config.admin_listen),
        Page::Apps => {
            let listener = config
                .listeners()
                .into_iter()
                .next()
                .unwrap_or(config.listen);
            if let Some(origin) = &config.public_origin {
                return format!("{}/", origin.trim_end_matches('/'));
            }
            if let Tls::Acme { domains, .. } = config.listener_tls(listener)
                && let Some(domain) = domains.first()
            {
                return format!(
                    "https://{}{}",
                    domain,
                    if listener.port() == 443 {
                        "/".into()
                    } else {
                        format!(":{}/", listener.port())
                    }
                );
            }
            let mut address = listener;
            if address.ip().is_unspecified() {
                address.set_ip(match address.ip() {
                    IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
                });
            }
            let scheme = if matches!(config.listener_tls(listener), Tls::Off {}) {
                "http"
            } else {
                "https"
            };
            format!("{scheme}://{address}/")
        }
    }
}
pub fn open(config: &Config, page: Page) -> Result<()> {
    let url = page_url(config, page);
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", &url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };
    command
        .spawn()
        .with_context(|| format!("Cannot open a browser. Open {url} manually"))?;
    Ok(())
}
async fn fetch(config: &Config, endpoint: &str, method: http::Method) -> Result<serde_json::Value> {
    let token = std::fs::read_to_string(config.state_dir.join("admin.token"))
        .context("Start the server first, using this configuration file")?;
    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http::<axum::body::Body>();
    let request = http::Request::builder()
        .method(method)
        .uri(format!("http://{}{endpoint}", config.admin_listen))
        .header(
            http::header::AUTHORIZATION,
            format!("Bearer {}", token.trim()),
        )
        .body(axum::body::Body::empty())?;
    tokio::time::timeout(Duration::from_secs(3), async {
        let response = client.request(request).await?;
        ensure!(
            response.status().is_success(),
            "Server returned HTTP {}. Check the configuration and admin token",
            response.status()
        );
        let bytes =
            axum::body::to_bytes(axum::body::Body::new(response.into_body()), 1024 * 1024).await?;
        Ok(serde_json::from_slice(&bytes)?)
    })
    .await
    .context("Server did not respond within three seconds")?
}
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect()
}
fn value<'a>(data: &'a serde_json::Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or("—")
}
fn number(data: &serde_json::Value, key: &str) -> u64 {
    data[key].as_u64().unwrap_or(0)
}

struct View {
    tab: usize,
    data: serde_json::Value,
    notice: String,
    online: bool,
    offset: usize,
}
impl View {
    async fn refresh(&mut self, config: &Config) {
        match fetch(config, "/api/status", http::Method::GET).await {
            Ok(data) => {
                self.data = data;
                self.online = true;
                self.notice = "Live status · refreshes every 2 seconds".into();
            }
            Err(e) => {
                self.online = false;
                self.notice = format!("Cannot connect: {e}. Start the server or check --config.");
            }
        }
    }
    fn draw(&self, frame: &mut Frame, config: &Config, path: &Path) {
        let teal = Color::Rgb(83, 211, 184);
        let muted = Color::Rgb(157, 176, 190);
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(3),
        ])
        .split(frame.area());
        let title = Line::from(vec![
            Span::styled(
                "  SHINYHARBOR  ",
                Style::default().fg(teal).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Your R application workspace"),
            Span::styled(
                if self.online {
                    "   ● CONNECTED"
                } else {
                    "   ○ OFFLINE"
                },
                Style::default().fg(if self.online { teal } else { Color::Yellow }),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(title).block(Block::default().borders(Borders::BOTTOM)),
            areas[0],
        );
        frame.render_widget(
            Tabs::new(["1 Overview", "2 Applications", "3 Help"])
                .select(self.tab)
                .highlight_style(Style::default().fg(teal).add_modifier(Modifier::BOLD)),
            areas[1],
        );
        let block = Block::bordered().title(match self.tab {
            1 => " APPLICATIONS ",
            2 => " KEYBOARD & NEXT STEPS ",
            _ => " SERVER OVERVIEW ",
        });
        if self.tab == 1 {
            let pools = self.data["pools"].as_array().cloned().unwrap_or_default();
            if pools.is_empty() {
                frame.render_widget(Paragraph::new("No running applications yet.\nOpen an app to start its R worker. Site directories discover apps when visited.\n\nPress O to open applications, or A for server settings.").wrap(Wrap{trim:false}).block(block), areas[2]);
            } else {
                let rows = pools.iter().skip(self.offset).map(|pool| {
                    Row::new(vec![
                        Cell::from(clean(value(pool, "name"))),
                        Cell::from(clean(value(pool, "mount"))),
                        Cell::from(number(pool, "workers").to_string()),
                        Cell::from(number(pool, "active_websockets").to_string()),
                        Cell::from(number(pool, "active_requests").to_string()),
                    ])
                });
                frame.render_widget(
                    Table::new(
                        rows,
                        [
                            Constraint::Percentage(30),
                            Constraint::Percentage(34),
                            Constraint::Length(8),
                            Constraint::Length(10),
                            Constraint::Length(10),
                        ],
                    )
                    .header(
                        Row::new(["App", "Route", "Workers", "Live", "Requests"])
                            .style(Style::default().fg(teal)),
                    )
                    .block(block),
                    areas[2],
                );
            }
        } else {
            let lines = if self.tab == 2 {
                vec!["A   Open the private administration page: explanations, validation and backups.".into(),
                    "D   Open the usage dashboard. O opens your applications.".into(),
                    "R   Refresh now. Tab / Left / Right switch panels; Up / Down scroll apps.".into(),
                    "Q / Esc / Ctrl+C   Close this monitor. The server keeps running.".into(),
                    "".into(), "Start a server: shinyharbor serve --config <file>".into(),
                    "Check R and Shiny: shinyharbor doctor --config <file>".into(),
                    "Unlock web admin: shinyharbor admin-token --config <file>".into(),
                    "".into(), "The app listener may use HTTPS; private administration uses loopback HTTP.".into(),
                    "For remote administration, use an SSH tunnel. See README.md.".into()]
            } else {
                let pools = self.data["pools"].as_array().cloned().unwrap_or_default();
                vec![
                    format!("Server version   {}", clean(value(&self.data, "version"))),
                    format!(
                        "App pools        {}     R workers  {}     Live connections  {}",
                        pools.len(),
                        pools.iter().map(|p| number(p, "workers")).sum::<u64>(),
                        pools
                            .iter()
                            .map(|p| number(p, "active_websockets"))
                            .sum::<u64>()
                    ),
                    "".into(),
                    format!("Applications     {}", page_url(config, Page::Apps)),
                    format!("Administration   {}", page_url(config, Page::Admin)),
                    format!("Usage dashboard  {}", page_url(config, Page::Dashboard)),
                    "".into(),
                    format!("Configuration    {}", path.display()),
                    format!("R executable     {}", config.rscript.display()),
                    "".into(),
                    "Press A for settings, certificates, app rules and configuration help.".into(),
                    "This is a monitor. Closing it does not stop your running server.".into(),
                ]
            };
            frame.render_widget(
                Paragraph::new(
                    lines
                        .into_iter()
                        .map(|s| Line::from(clean(&s)))
                        .collect::<Vec<_>>(),
                )
                .wrap(Wrap { trim: false })
                .block(block),
                areas[2],
            );
        }
        frame.render_widget(
            Paragraph::new(clean(&self.notice))
                .style(Style::default().fg(muted))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::TOP)
                        .title(" A Admin · D Dashboard · O Apps · R Refresh · Q Quit "),
                ),
            areas[3],
        );
    }
}

pub async fn run(path: &Path, once: bool, width: u16) -> Result<()> {
    let config = Config::load(path)?;
    let mut view = View {
        tab: 0,
        data: serde_json::Value::Null,
        notice: String::new(),
        online: false,
        offset: 0,
    };
    view.refresh(&config).await;
    if once {
        let mut terminal = Terminal::new(TestBackend::new(width, 26))?;
        terminal.draw(|frame| view.draw(frame, &config, path))?;
        let buffer = terminal.backend().buffer();
        for y in 0..buffer.area.height {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            println!("{}", line.trim_end());
        }
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "TUI requires an interactive terminal. Use `shinyharbor tui --once` for a plain-text snapshot"
    );
    let mut terminal = ratatui::try_init()?;
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }
    let _restore = Restore;
    let mut refreshed = Instant::now();
    loop {
        terminal.draw(|frame| view.draw(frame, &config, path))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let code = match key.code {
                KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
                code => code,
            };
            match code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Tab | KeyCode::Right => view.tab = (view.tab + 1) % 3,
                KeyCode::BackTab | KeyCode::Left => view.tab = (view.tab + 2) % 3,
                KeyCode::Char(c @ '1'..='3') => view.tab = c as usize - '1' as usize,
                KeyCode::Up => view.offset = view.offset.saturating_sub(1),
                KeyCode::Down => {
                    let count = view.data["pools"].as_array().map_or(0, Vec::len);
                    view.offset = (view.offset + 1).min(count.saturating_sub(1));
                }
                KeyCode::Char('r') => {
                    view.refresh(&config).await;
                    refreshed = Instant::now();
                }
                KeyCode::Char(c @ ('a' | 'd' | 'o')) => {
                    let page = match c {
                        'a' => Page::Admin,
                        'd' => Page::Dashboard,
                        _ => Page::Apps,
                    };
                    view.notice = match open(&config, page) {
                        Ok(()) => format!("Opened {}", page_url(&config, page)),
                        Err(e) => format!("{e:#}"),
                    };
                }
                _ => {}
            }
        }
        if refreshed.elapsed() >= Duration::from_secs(2) {
            view.refresh(&config).await;
            refreshed = Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_follow_tls_custom_ports_and_ipv6() {
        let mut config = Config {
            listen: "[::]:3131".parse().unwrap(),
            admin_listen: "[::1]:3232".parse().unwrap(),
            ..Config::default()
        };
        assert_eq!(page_url(&config, Page::Apps), "http://[::1]:3131/");
        config.tls = Tls::Pem {
            cert: "cert.pem".into(),
            key: "key.pem".into(),
        };
        assert_eq!(page_url(&config, Page::Apps), "https://[::1]:3131/");
        assert_eq!(page_url(&config, Page::Admin), "http://[::1]:3232/admin");
    }
    #[test]
    fn renders_at_small_terminal_sizes_and_removes_controls() {
        let view = View {
            tab: 0,
            data: serde_json::Value::Null,
            notice: "Offline".into(),
            online: false,
            offset: 0,
        };
        for (width, height) in [(40, 12), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| view.draw(frame, &Config::default(), Path::new("harbor.toml")))
                .unwrap();
        }
        assert_eq!(clean("label\u{1b}\n"), "label");
    }
}
