//! Optional bounded HTTP access logging, separate from aggregate usage analytics.
use anyhow::{Result, bail, ensure};
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use http::{HeaderMap, Request, Version};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncWriteExt, BufWriter},
    sync::{mpsc, oneshot},
};

#[derive(Clone)]
enum Part {
    Text(String),
    Token(String, Option<String>),
}
fn compile(format: &str) -> Result<Vec<Part>> {
    let format = match format {
        "default" => {
            ":remote-addr - :remote-user [:date] \":method :url HTTP/:http-version\" :status :res[content-length] \":referrer\" \":user-agent\""
        }
        "combined" => {
            ":remote-addr - :remote-user [:date[clf]] \":method :url HTTP/:http-version\" :status :res[content-length] \":referrer\" \":user-agent\""
        }
        "common" => {
            ":remote-addr - :remote-user [:date[clf]] \":method :url HTTP/:http-version\" :status :res[content-length]"
        }
        "short" => {
            ":remote-addr :remote-user :method :url HTTP/:http-version :status :res[content-length] - :response-time ms"
        }
        "tiny" => ":method :url :status :res[content-length] - :response-time ms",
        "dev" => ":method :url :status :response-time ms - :res[content-length]",
        other => other,
    };
    ensure!(
        format.len() <= 4096 && !format.chars().any(char::is_control),
        "Access log format must be at most 4096 bytes without control characters"
    );
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < format.len() {
        let rest = &format[pos..];
        let Some(start) = rest.find(':') else {
            out.push(Part::Text(rest.into()));
            break;
        };
        out.push(Part::Text(rest[..start].into()));
        pos += start + 1;
        let count = format[pos..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
            .count();
        if count == 0 {
            out.push(Part::Text(":".into()));
            continue;
        }
        let name = &format[pos..pos + count];
        pos += count;
        let arg = if format[pos..].starts_with('[') {
            let end = format[pos + 1..]
                .find(']')
                .ok_or_else(|| anyhow::anyhow!("Unclosed access log token argument"))?;
            let arg = format[pos + 1..pos + 1 + end].to_owned();
            pos += end + 2;
            Some(arg)
        } else {
            None
        };
        match name {
            "req" | "res" => {
                arg.as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Header log token requires a name"))?
                    .parse::<http::HeaderName>()?;
            }
            "date" => ensure!(
                matches!(arg.as_deref(), None | Some("web" | "clf" | "iso")),
                "Unknown log date format"
            ),
            "response-time" | "total-time" => {
                if let Some(arg) = &arg {
                    ensure!(
                        arg.parse::<usize>().is_ok_and(|n| n <= 6),
                        "Log timing precision must be 0..6"
                    );
                }
            }
            "remote-addr" | "remote-user" | "method" | "url" | "http-version" | "status"
            | "referrer" | "user-agent" => ensure!(
                arg.is_none(),
                "Unexpected argument for access log token {name}"
            ),
            _ => bail!("Unsupported access log token {name}"),
        }
        out.push(Part::Token(name.into(), arg));
    }
    Ok(out)
}
pub fn validate(format: &str) -> Result<()> {
    compile(format).map(|_| ())
}
enum Message {
    Line(String),
    Reopen,
    Stop(oneshot::Sender<()>),
}
pub struct AccessLog {
    format: Vec<Part>,
    sender: mpsc::Sender<Message>,
    pub dropped: AtomicU64,
}
async fn open(path: &std::path::Path) -> std::io::Result<BufWriter<tokio::fs::File>> {
    let mut options = tokio::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    options.mode(0o640);
    options.open(path).await.map(BufWriter::new)
}
impl AccessLog {
    pub async fn start(config: &crate::config::AccessLog) -> Result<Arc<Self>> {
        let format = compile(&config.format)?;
        let file = open(&config.path).await?;
        let (sender, receiver) = mpsc::channel(1024);
        let log = Arc::new(Self {
            format,
            sender,
            dropped: AtomicU64::new(0),
        });
        let path = config.path.clone();
        tokio::spawn(async move {
            if let Err(error) = writer(path, file, receiver).await {
                tracing::error!(%error, "Access log writer stopped");
            }
        });
        Ok(log)
    }
    pub async fn reopen(&self) -> Result<()> {
        self.sender.send(Message::Reopen).await.map_err(Into::into)
    }
    pub async fn shutdown(&self) {
        let (done, completed) = oneshot::channel();
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            if self.sender.send(Message::Stop(done)).await.is_ok() {
                let _ = completed.await;
            }
        })
        .await;
    }
    fn record(&self, record: Record) {
        let mut line = String::new();
        for part in &self.format {
            match part {
                Part::Text(text) => line.push_str(text),
                Part::Token(name, arg) => {
                    let value = record.value(name, arg.as_deref());
                    for ch in value.chars() {
                        // User-controlled fields cannot create extra lines or quoted fields.
                        match ch {
                            '\\' => line.push_str("\\\\"),
                            '"' => line.push_str("\\\""),
                            c if c.is_control() => line.push_str(&format!("\\u{:04x}", c as u32)),
                            c => line.push(c),
                        }
                        if line.len() >= 16 * 1024 {
                            break;
                        }
                    }
                }
            }
            if line.len() >= 16 * 1024 {
                break;
            }
        }
        line.push('\n');
        if self.sender.try_send(Message::Line(line)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
async fn writer(
    path: PathBuf,
    mut file: BufWriter<tokio::fs::File>,
    mut receiver: mpsc::Receiver<Message>,
) -> std::io::Result<()> {
    let mut flush = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            message = receiver.recv() => match message {
                Some(Message::Line(line)) => file.write_all(line.as_bytes()).await?,
                Some(Message::Reopen) => { file.flush().await?; match open(&path).await { Ok(next) => file = next, Err(error) => tracing::error!(%error, "Cannot reopen access log; retaining previous file") } }
                Some(Message::Stop(done)) => { file.flush().await?; let _ = done.send(()); return Ok(()); }
                None => { file.flush().await?; return Ok(()); }
            },
            _ = flush.tick() => file.flush().await?,
        }
    }
}
struct Record {
    request: http::request::Parts,
    peer: String,
    start: Instant,
    date: DateTime<Utc>,
    status: u16,
    headers: HeaderMap,
    response_time: Duration,
}
fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_owned()
}
impl Record {
    fn value(&self, name: &str, arg: Option<&str>) -> String {
        match name {
            "remote-addr" => self.peer.clone(),
            // Like the upstream access logger, this field identifies the presented Basic username;
            // it does not indicate that authentication succeeded.
            "remote-user" => self
                .request
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split_once(' '))
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("basic"))
                .and_then(|(_, value)| base64::engine::general_purpose::STANDARD.decode(value).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|value| value.split_once(':').map(|(user, _)| user.to_owned()))
                .unwrap_or_else(|| "-".into()),
            "method" => self.request.method.to_string(),
            "url" => self.request.uri.to_string(),
            "http-version" => match self.request.version {
                Version::HTTP_09 => "0.9",
                Version::HTTP_10 => "1.0",
                Version::HTTP_11 => "1.1",
                Version::HTTP_2 => "2.0",
                Version::HTTP_3 => "3.0",
                _ => "-",
            }
            .into(),
            "status" => self.status.to_string(),
            "referrer" => self
                .request
                .headers
                .get("referer")
                .or_else(|| self.request.headers.get("referrer"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .into(),
            "user-agent" => header(&self.request.headers, "user-agent"),
            "req" => header(&self.request.headers, arg.unwrap_or("")),
            "res" => header(&self.headers, arg.unwrap_or("")),
            "response-time" | "total-time" => {
                let duration = if name == "total-time" {
                    self.start.elapsed()
                } else {
                    self.response_time
                };
                format!(
                    "{:.*}",
                    arg.and_then(|s| s.parse().ok()).unwrap_or(3),
                    duration.as_secs_f64() * 1000.0
                )
            }
            "date" => match arg {
                Some("clf") => self.date.format("%d/%b/%Y:%H:%M:%S +0000").to_string(),
                Some("iso") => self
                    .date
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                _ => self.date.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
            },
            _ => "-".into(),
        }
    }
}
pub async fn middleware(
    State(log): State<Arc<AccessLog>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|| "-".into());
    let (parts, body) = req.into_parts();
    let request = Request::from_parts(parts.clone(), body);
    let start = Instant::now();
    let date = Utc::now();
    let response = next.run(request).await;
    let record = Record {
        request: parts,
        peer,
        start,
        date,
        status: response.status().as_u16(),
        headers: response.headers().clone(),
        response_time: start.elapsed(),
    };
    response.map(|body| {
        Body::new(LoggedBody {
            body,
            log,
            record: Some(record),
        })
    })
}
struct LoggedBody {
    body: Body,
    log: Arc<AccessLog>,
    record: Option<Record>,
}
impl Drop for LoggedBody {
    fn drop(&mut self) {
        if let Some(record) = self.record.take() {
            self.log.record(record);
        }
    }
}
impl HttpBody for LoggedBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let result = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(result, Poll::Ready(None | Some(Err(_))))
            && let Some(record) = self.record.take()
        {
            self.log.record(record);
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}
