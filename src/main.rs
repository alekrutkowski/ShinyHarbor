use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use shinyharbor::{config::Config, security, server};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(version, about = "Host R Shiny apps with HTTPS, process pools and a private usage dashboard", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Write a local configuration and working example without overwriting files.
    Init {
        #[arg(default_value = ".")]
        directory: PathBuf,
    },
    /// Start the application and private admin listeners.
    Serve {
        #[arg(short, long, default_value = "harbor.toml")]
        config: PathBuf,
    },
    /// Validate configuration, paths, routes and credentials without starting R.
    Check {
        #[arg(short, long, default_value = "harbor.toml")]
        config: PathBuf,
    },
    /// Reload application routes and access rules without closing existing sessions.
    Reload {
        #[arg(short, long, default_value = "harbor.toml")]
        config: PathBuf,
    },
    /// Check configuration and report installed R/package versions.
    Doctor {
        #[arg(short, long, default_value = "harbor.toml")]
        config: PathBuf,
    },
    /// Print the local dashboard token. Treat the output as a secret.
    AdminToken {
        #[arg(short, long, default_value = "harbor.toml")]
        config: PathBuf,
    },
    /// Hash a password read from the terminal, never from an argument.
    HashPassword,
    #[command(hide = true)]
    InternalLogWriter {
        #[arg(long)]
        prefix: PathBuf,
        #[arg(long)]
        mode: String,
        #[arg(long)]
        preserve: bool,
    },
    /// Entry point used only by Windows Service Control Manager.
    #[cfg(windows)]
    Service {
        #[arg(short, long)]
        config: PathBuf,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    #[cfg(windows)]
    if let Some(Command::Service { config }) = &cli.command {
        return windows::dispatch(config);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "shinyharbor=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match cli.command.unwrap_or(Command::Serve {
        config: "harbor.toml".into(),
    }) {
        Command::Init { directory } => initialize(&directory),
        Command::Check { config } => {
            let c = Config::load(&config)?;
            println!(
                "Configuration valid: {} native app(s), {} site(s), {} redirect(s), {} upstream route(s).",
                c.apps.len(),
                c.sites.len(),
                c.redirects.len(),
                c.upstreams.len()
            );
            Ok(())
        }
        Command::AdminToken { config } => {
            let c = Config::load(&config)?;
            println!("{}", server::admin_token(&c.state_dir)?);
            Ok(())
        }
        Command::HashPassword => hash_password(),
        Command::InternalLogWriter {
            prefix,
            mode,
            preserve,
        } => shinyharbor::user_log::run(&prefix, &mode, preserve),
        Command::Reload { config } => runtime()?.block_on(reload(Config::load(&config)?)),
        Command::Doctor { config } => runtime()?.block_on(doctor(Config::load(&config)?)),
        Command::Serve { config } => runtime()?.block_on(async move {
            let stop = CancellationToken::new();
            let signal_stop = stop.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                signal_stop.cancel();
            });
            server::run_file(&config, stop).await
        }),
        #[cfg(windows)]
        Command::Service { .. } => {
            unreachable!("Service dispatch is handled before console logging")
        }
    }
}
fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}
async fn reload(config: Config) -> Result<()> {
    let token = std::fs::read_to_string(config.state_dir.join("admin.token"))?;
    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http::<axum::body::Body>();
    let request = http::Request::builder()
        .method("POST")
        .uri(format!("http://{}/api/reload", config.admin_listen))
        .header(
            http::header::AUTHORIZATION,
            format!("Bearer {}", token.trim()),
        )
        .body(axum::body::Body::empty())?;
    let response = tokio::time::timeout(Duration::from_secs(30), client.request(request)).await??;
    ensure!(
        response.status().is_success(),
        "Reload rejected ({}). Check the server log; the previous configuration remains active.",
        response.status()
    );
    println!("Application configuration reloaded. Existing sessions remain connected.");
    Ok(())
}
fn initialize(directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("harbor.toml");
    ensure!(
        !path.exists(),
        "{} already exists; no files were overwritten",
        path.display()
    );
    let app = directory.join("apps").join("hello");
    std::fs::create_dir_all(&app)?;
    write_new(&app.join("app.R"), include_str!("../examples/hello/app.R"))?;
    write_new(&path, include_str!("../examples/init.toml"))?;
    let c = Config::load(&path)?;
    let _ = server::admin_token(&c.state_dir)?;
    println!(
        "Created {}\nRun: shinyharbor doctor --config \"{}\"\nThen: shinyharbor serve --config \"{}\"\nApplication: http://127.0.0.1:3838/hello/\nDashboard: http://127.0.0.1:3839/\nUnlock it with: shinyharbor admin-token --config \"{}\"",
        path.display(),
        path.display(),
        path.display(),
        path.display()
    );
    Ok(())
}
fn write_new(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("Refusing to overwrite {}", path.display()))?;
    file.write_all(text.as_bytes())?;
    Ok(())
}
fn hash_password() -> Result<()> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    let password = rpassword::prompt_password("Password: ")?;
    ensure!(password.len() >= 12, "Use a password of at least 12 bytes");
    let confirmation = rpassword::prompt_password("Confirm password: ")?;
    ensure!(
        security::constant_eq(&password, &confirmation),
        "Passwords did not match"
    );
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Cannot hash password: {e}"))?;
    println!("{hash}");
    Ok(())
}
async fn doctor(c: Config) -> Result<()> {
    println!(
        "Configuration: valid\nServer target: {}-{}\nCompatibility certification: NOT established",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let mut executables: Vec<_> = c
        .apps
        .iter()
        .chain(c.sites.iter().map(|s| &s.app_defaults))
        .map(|a| a.rscript.as_ref().unwrap_or(&c.rscript))
        .collect();
    executables.sort();
    executables.dedup();
    for exe in executables {
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(["--vanilla", "-e", "cat(R.version.string, '\\n'); stopifnot(requireNamespace('shiny', quietly=TRUE)); cat('shiny ', as.character(packageVersion('shiny')), '\\n', sep='')"])
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .context("R check timed out")??;
        print!(
            "{}: {}",
            exe.display(),
            String::from_utf8_lossy(&output.stdout)
        );
        ensure!(
            output.status.success(),
            "R/shiny check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if c.apps.is_empty() && c.sites.is_empty() {
        println!("No native applications/sites configured: no local R executable required.");
    }
    for up in &c.upstreams {
        println!(
            "Upstream {}: {} backend(s), original request paths retained",
            up.name,
            up.backends.len()
        );
    }
    println!(
        "Doctor does not certify application behavior, upstream availability, DNS, certificates, or compatibility."
    );
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::OsString, sync::OnceLock};
    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
    };
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    // The SCM entry-point macro contains FFI glue; no handwritten unsafe code.
    #[allow(unsafe_code)]
    mod entry {
        use super::*;
        define_windows_service!(ffi_main, service_main);
        pub(super) fn start() -> windows_service::Result<()> {
            service_dispatcher::start("ShinyHarbor", ffi_main)
        }
    }
    pub fn dispatch(config: &Path) -> Result<()> {
        CONFIG
            .set(std::fs::canonicalize(config)?)
            .map_err(|_| anyhow::anyhow!("Service already configured"))?;
        entry::start()?;
        Ok(())
    }
    fn service_main(_args: Vec<OsString>) {
        if let Err(error) = service_run() {
            eprintln!("ShinyHarbor service failed: {error:#}");
        }
    }
    fn service_run() -> Result<()> {
        let config_path = CONFIG.get().context("Missing service configuration")?;
        let config = Config::load(config_path)?;
        security::private_dir(&config.state_dir)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(config.state_dir.join("service.log"))?;
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(log))
            .try_init();
        let stop = CancellationToken::new();
        let stop2 = stop.clone();
        let handle =
            service_control_handler::register("ShinyHarbor", move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    stop2.cancel();
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            })?;
        let status = |current_state, exit_code| ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state,
            controls_accepted: if current_state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(exit_code),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        };
        handle.set_service_status(status(ServiceState::Running, 0))?;
        let result = runtime()?.block_on(server::run_file(config_path, stop));
        if let Err(error) = &result {
            tracing::error!(%error, "Service stopped with an error");
        }
        handle.set_service_status(status(ServiceState::Stopped, u32::from(result.is_err())))?;
        result
    }
}
