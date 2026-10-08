//! Log I/O performed under an R worker's Unix credentials, including creation,
//! rotation and deletion on root-squashed filesystems. The gateway never changes UID.
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::task::TaskTracker;

enum Message {
    Data(u8, Vec<u8>),
    Finish(bool),
}
pub struct UserLog {
    sender: mpsc::Sender<Message>,
    tasks: TaskTracker,
}
impl UserLog {
    pub async fn start(
        prefix: &Path,
        mode: &str,
        preserve: bool,
        identity: &crate::identity::Identity,
        tasks: &TaskTracker,
    ) -> Result<Self> {
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command
            .arg("internal-log-writer")
            .arg("--prefix")
            .arg(prefix)
            .arg("--mode")
            .arg(mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if preserve {
            command.arg("--preserve");
        }
        identity.apply(&mut command);
        let mut child = command
            .spawn()
            .context("Cannot start worker-owned log writer")?;
        let mut ready = [0];
        tokio::time::timeout(
            Duration::from_secs(10),
            child
                .stdout
                .as_mut()
                .context("Missing writer readiness pipe")?
                .read_exact(&mut ready),
        )
        .await??;
        ensure!(ready == [1], "Worker-owned log writer failed to initialize");
        drop(child.stdout.take());
        let mut input = child.stdin.take().context("Missing writer input pipe")?;
        let (sender, mut receiver) = mpsc::channel(32);
        tasks.spawn(async move {
            let result: Result<()> = async {
                while let Some(message) = receiver.recv().await {
                    let (channel, bytes, finish) = match message {
                        Message::Data(channel, bytes) => (channel, bytes, false),
                        Message::Finish(remove) => (2, vec![u8::from(remove)], true),
                    };
                    input.write_u8(channel).await?;
                    input.write_u32(bytes.len() as u32).await?;
                    input.write_all(&bytes).await?;
                    if finish {
                        break;
                    }
                }
                drop(input);
                let status = child.wait().await?;
                ensure!(
                    status.success(),
                    "Worker-owned log writer exited with {status}"
                );
                Ok(())
            }
            .await;
            if let Err(error) = result {
                tracing::error!(%error, "Worker-owned log writer failed");
            }
        });
        Ok(Self {
            sender,
            tasks: tasks.clone(),
        })
    }
    pub async fn write(&self, channel: u8, bytes: &[u8]) -> Result<()> {
        self.sender
            .send(Message::Data(channel, bytes.to_vec()))
            .await
            .map_err(Into::into)
    }
    pub fn finish(&self, remove: bool) {
        let sender = self.sender.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            self.tasks.spawn_on(
                async move {
                    let _ = sender.send(Message::Finish(remove)).await;
                },
                &runtime,
            );
        } else {
            let _ = sender.try_send(Message::Finish(remove));
        }
    }
}

struct LogFile {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
    mode: u32,
}
fn open(path: &Path, mode: u32) -> Result<std::fs::File> {
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
    Ok(file)
}
impl LogFile {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        if self.size + bytes.len() as u64 > 10 * 1024 * 1024 {
            self.file.flush()?;
            // This helper is used for changed Unix identities. Rename keeps the
            // old descriptor valid until it is replaced, with no delete/reopen gap.
            let old = self.path.with_extension("log.1");
            match std::fs::remove_file(&old) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            std::fs::rename(&self.path, old)?;
            self.file = open(&self.path, self.mode)?;
            self.size = 0;
        }
        self.file.write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }
}

/// Private subprocess entry point. It runs synchronously without a Tokio thread
/// pool, and receives only bounded data frames and a final retention decision.
pub fn run(prefix: &Path, mode: &str, preserve: bool) -> Result<()> {
    use std::io::{Read, Write};
    let mode = crate::config::log_mode(mode)?;
    let parent = prefix.parent().context("Log path has no parent")?;
    if !parent.exists() {
        crate::security::private_dir(parent)?;
    }
    ensure!(
        std::fs::symlink_metadata(parent)?.is_dir(),
        "Log directory must be a real directory"
    );
    let mut logs = Vec::new();
    for stream in ["out", "err"] {
        let path = PathBuf::from(format!("{}-{stream}.log", prefix.display()));
        let file = open(&path, mode)?;
        if preserve {
            open(&path.with_extension("log.keep"), mode)?;
        }
        logs.push(LogFile {
            path,
            file,
            size: 0,
            mode,
        });
    }
    std::io::stdout().write_all(&[1])?;
    std::io::stdout().flush()?;
    let mut input = std::io::stdin().lock();
    let mut remove = false;
    loop {
        let mut channel = [0];
        if input.read(&mut channel)? == 0 {
            break;
        }
        let mut size = [0; 4];
        input.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size) as usize;
        ensure!(
            size <= 16 * 1024 && channel[0] <= 2,
            "Invalid log writer frame"
        );
        let mut bytes = vec![0; size];
        input.read_exact(&mut bytes)?;
        if channel[0] == 2 {
            ensure!(bytes.len() == 1, "Invalid log retention frame");
            remove = bytes[0] == 1 && !preserve;
            break;
        }
        logs[usize::from(channel[0])].write(&bytes)?;
    }
    for mut log in logs {
        log.file.flush()?;
        drop(log.file);
        if remove {
            std::fs::remove_file(&log.path)?;
            let _ = std::fs::remove_file(log.path.with_extension("log.1"));
        }
    }
    Ok(())
}
