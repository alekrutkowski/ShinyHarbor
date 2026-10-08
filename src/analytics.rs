use crate::{
    config::Analytics as AnalyticsConfig,
    security::{Secret, epoch_seconds},
};
use anyhow::{Context, Result};
use http::HeaderMap;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::IpAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

#[derive(Clone, Default, Debug)]
pub struct Event {
    pub time: u64,
    pub app: String,
    pub country: String,
    pub referrer: String,
    pub visitor: String,
    pub request: u64,
    pub page: u64,
    pub websocket: u64,
    pub error: u64,
    pub interrupted: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub header_ms: u64,
    pub websocket_ms: u64,
}
enum Message {
    Event(Event),
    Snapshot(u32, oneshot::Sender<Result<Value>>),
    Shutdown(oneshot::Sender<()>),
}
pub struct Analytics {
    sender: mpsc::SyncSender<Message>,
    pub dropped: AtomicU64,
    enabled: bool,
    secret: Secret,
    country: Option<maxminddb::Reader<Vec<u8>>>,
}
#[derive(Deserialize)]
struct CountryRecord {
    country: Option<CountryCode>,
}
#[derive(Deserialize)]
struct CountryCode {
    iso_code: Option<String>,
}
impl Analytics {
    pub fn start(config: &AnalyticsConfig, path: &Path, secret: Secret) -> Result<Arc<Self>> {
        let country = config
            .country_database
            .as_ref()
            .map(maxminddb::Reader::open_readfile)
            .transpose()
            .context("Cannot read configured country database")?;
        let conn = Connection::open(path).context("Cannot open analytics database")?;
        setup(&conn)?;
        let (sender, receiver) = mpsc::sync_channel(config.queue_capacity);
        let retention = config.retention_days;
        std::thread::Builder::new()
            .name("harbor-analytics".into())
            .spawn(move || {
                if let Err(e) = database_loop(conn, receiver, retention) {
                    tracing::error!(error = %e, "Analytics database writer stopped");
                }
            })?;
        Ok(Arc::new(Self {
            sender,
            dropped: AtomicU64::new(0),
            enabled: config.enabled,
            secret,
            country,
        }))
    }
    pub fn event(&self, ip: IpAddr, headers: &HeaderMap, app: &str) -> Option<Event> {
        if !self.enabled
            || headers.get("dnt").is_some_and(|v| v == "1")
            || headers.get("sec-gpc").is_some_and(|v| v == "1")
        {
            return None;
        }
        let country = self
            .country
            .as_ref()
            .and_then(|reader| reader.lookup(ip).ok())
            .and_then(|lookup| lookup.decode::<CountryRecord>().ok().flatten())
            .and_then(|r| r.country)
            .and_then(|c| c.iso_code)
            .filter(|s| s.len() == 2)
            .unwrap_or_else(|| "ZZ".into());
        Some(Event {
            time: epoch_seconds(),
            app: app.into(),
            country,
            visitor: self.secret.visitor(ip),
            referrer: crate::security::referrer_origin(headers),
            ..Event::default()
        })
    }
    pub fn record(&self, event: Option<Event>) {
        if let Some(event) = event
            && self.sender.try_send(Message::Event(event)).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub async fn snapshot(&self, days: u32) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .try_send(Message::Snapshot(days.clamp(1, 366), tx))
            .map_err(|_| anyhow::anyhow!("Analytics queue is busy or unavailable"))?;
        let mut value = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .context("Analytics query timed out")???;
        value["dropped_events"] = self.dropped.load(Ordering::Relaxed).into();
        value["enabled"] = self.enabled.into();
        value["country_database_loaded"] = self.country.is_some().into();
        Ok(value)
    }
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        let mut message = Message::Shutdown(tx);
        loop {
            match self.sender.try_send(message) {
                Ok(()) => {
                    let _ = tokio::time::timeout(Duration::from_secs(10), rx).await;
                    break;
                }
                Err(mpsc::TrySendError::Full(returned)) => {
                    message = returned;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
    }
}
fn setup(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    anyhow::ensure!(
        version <= 1,
        "Database was created by a newer ShinyHarbor version"
    );
    conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;
        PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS traffic (
          hour INTEGER NOT NULL, app TEXT NOT NULL, country TEXT NOT NULL, referrer TEXT NOT NULL,
          requests INTEGER NOT NULL, pages INTEGER NOT NULL, websockets INTEGER NOT NULL,
          errors INTEGER NOT NULL, interrupted INTEGER NOT NULL,
          bytes_in INTEGER NOT NULL, bytes_out INTEGER NOT NULL, header_ms INTEGER NOT NULL, websocket_ms INTEGER NOT NULL,
          PRIMARY KEY(hour,app,country,referrer)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS visitors (
          day INTEGER NOT NULL, app TEXT NOT NULL, visitor TEXT NOT NULL,
          PRIMARY KEY(day,app,visitor)
        ) WITHOUT ROWID;
        PRAGMA user_version=1;")?;
    Ok(())
}
fn flush(conn: &mut Connection, events: &mut Vec<Event>) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut traffic = tx.prepare_cached("INSERT INTO traffic VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
          ON CONFLICT(hour,app,country,referrer) DO UPDATE SET
          requests=requests+excluded.requests, pages=pages+excluded.pages, websockets=websockets+excluded.websockets,
          errors=errors+excluded.errors, interrupted=interrupted+excluded.interrupted,
          bytes_in=bytes_in+excluded.bytes_in, bytes_out=bytes_out+excluded.bytes_out,
          header_ms=header_ms+excluded.header_ms, websocket_ms=websocket_ms+excluded.websocket_ms")?;
        let mut visitors = tx.prepare_cached("INSERT OR IGNORE INTO visitors VALUES (?1,?2,?3)")?;
        for e in events.iter() {
            traffic.execute(params![
                e.time / 3600,
                e.app,
                e.country,
                e.referrer,
                e.request,
                e.page,
                e.websocket,
                e.error,
                e.interrupted,
                e.bytes_in,
                e.bytes_out,
                e.header_ms,
                e.websocket_ms
            ])?;
            if e.page > 0 {
                visitors.execute(params![e.time / 86400, e.app, e.visitor])?;
            }
        }
    }
    tx.commit()?;
    events.clear();
    Ok(())
}
fn database_loop(mut conn: Connection, rx: mpsc::Receiver<Message>, retention: u32) -> Result<()> {
    let mut events = Vec::with_capacity(500);
    let mut last_flush = Instant::now();
    let mut last_cleanup = Instant::now() - Duration::from_secs(3601);
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Message::Event(event)) => events.push(event),
            Ok(Message::Snapshot(days, reply)) => {
                flush(&mut conn, &mut events)?;
                let _ = reply.send(query(&conn, days));
            }
            Ok(Message::Shutdown(reply)) => {
                flush(&mut conn, &mut events)?;
                conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
                let _ = reply.send(());
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush(&mut conn, &mut events)?;
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if events.len() >= 500 || last_flush.elapsed() >= Duration::from_secs(1) {
            flush(&mut conn, &mut events)?;
            last_flush = Instant::now();
        }
        if last_cleanup.elapsed() >= Duration::from_secs(3600) {
            let cutoff = epoch_seconds().saturating_sub(u64::from(retention) * 86400);
            conn.execute("DELETE FROM traffic WHERE hour < ?1", [cutoff / 3600])?;
            conn.execute("DELETE FROM visitors WHERE day < ?1", [cutoff / 86400])?;
            conn.execute_batch("PRAGMA incremental_vacuum(1000); PRAGMA wal_checkpoint(PASSIVE)")?;
            last_cleanup = Instant::now();
        }
    }
    Ok(())
}
#[derive(Serialize)]
struct AppRow {
    app: String,
    requests: u64,
    pages: u64,
    websockets: u64,
    errors: u64,
    interrupted: u64,
    bytes_in: u64,
    bytes_out: u64,
    header_ms: u64,
    websocket_ms: u64,
    visitor_days: u64,
}
fn query(conn: &Connection, days: u32) -> Result<Value> {
    let cutoff = epoch_seconds().saturating_sub(u64::from(days) * 86400);
    let mut stmt = conn.prepare(
        "SELECT app, SUM(requests), SUM(pages), SUM(websockets), SUM(errors),
       SUM(interrupted), SUM(bytes_in), SUM(bytes_out), SUM(header_ms), SUM(websocket_ms),
       (SELECT COUNT(*) FROM visitors v WHERE v.app=t.app AND day>=?2)
       FROM traffic t WHERE hour>=?1 GROUP BY app ORDER BY SUM(pages) DESC",
    )?;
    let apps = stmt
        .query_map(params![cutoff / 3600, cutoff / 86400], |r| {
            Ok(AppRow {
                app: r.get(0)?,
                requests: r.get(1)?,
                pages: r.get(2)?,
                websockets: r.get(3)?,
                errors: r.get(4)?,
                interrupted: r.get(5)?,
                bytes_in: r.get(6)?,
                bytes_out: r.get(7)?,
                header_ms: r.get(8)?,
                websocket_ms: r.get(9)?,
                visitor_days: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT hour, SUM(requests), SUM(pages) FROM traffic WHERE hour>=?1 GROUP BY hour ORDER BY hour")?;
    let series = stmt.query_map([cutoff / 3600], |r| Ok(json!({"hour":r.get::<_,u64>(0)?, "requests":r.get::<_,u64>(1)?, "pages":r.get::<_,u64>(2)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT country, SUM(pages) FROM traffic WHERE hour>=?1 GROUP BY country ORDER BY SUM(pages) DESC LIMIT 30")?;
    let countries = stmt
        .query_map([cutoff / 3600], |r| {
            Ok(json!({"code":r.get::<_,String>(0)?, "pages":r.get::<_,u64>(1)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT referrer, SUM(pages) FROM traffic WHERE hour>=?1 GROUP BY referrer ORDER BY SUM(pages) DESC LIMIT 20")?;
    let referrers = stmt
        .query_map([cutoff / 3600], |r| {
            Ok(json!({"origin":r.get::<_,String>(0)?, "pages":r.get::<_,u64>(1)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({ "days":days, "generated_at":epoch_seconds(), "apps":apps, "series":series,
        "countries":countries, "referrers":referrers,
        "definitions": {"pages":"Successful HTML document responses; not unique people", "websockets":"Successful WebSocket upgrades, including reconnects; not SockJS session counts", "visitor_days":"Distinct daily keyed IP hashes per app, summed across days; not unique users", "countries":"Estimated IP country from local database; ZZ means unknown"} }),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregation_and_daily_visitors() {
        let mut c = Connection::open_in_memory().unwrap();
        setup(&c).unwrap();
        let e = Event {
            time: epoch_seconds(),
            app: "hello".into(),
            country: "NL".into(),
            visitor: "rotating-id".into(),
            request: 1,
            page: 1,
            ..Event::default()
        };
        flush(&mut c, &mut vec![e.clone(), e]).unwrap();
        let v = query(&c, 1).unwrap();
        assert_eq!(v["apps"][0]["requests"], 2);
        assert_eq!(v["apps"][0]["visitor_days"], 1);
        let columns: String = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='traffic'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!columns.contains("ip_address"));
    }
}
