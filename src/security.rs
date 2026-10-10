use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use http::{HeaderMap, HeaderName, header};
use ipnet::IpNet;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::IpAddr,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

/// Never allow a client-selected header to replace transport or identity metadata.
pub fn validate_forwarded_headers(names: &[String]) -> Result<()> {
    ensure!(
        names.len() <= 64,
        "At most 64 whitelist_headers are supported"
    );
    for raw in names {
        let name =
            HeaderName::from_bytes(raw.as_bytes()).context("Invalid whitelist_headers name")?;
        ensure!(
            forwardable_header(&name),
            "Header {name} is managed by the server and cannot be whitelisted"
        );
    }
    Ok(())
}
fn forwardable_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "authorization"
            | "proxy-authorization"
            | "connection"
            | "upgrade"
            | "host"
            | "content-length"
            | "transfer-encoding"
            | "te"
            | "trailer"
            | "keep-alive"
            | "forwarded"
            | "x-real-ip"
            | "x-harbor-user"
            | "shiny-shared-secret"
            | "shiny-server-credentials"
    ) && !name.as_str().starts_with("x-forwarded-")
        && !name.as_str().starts_with("sec-websocket-")
}
pub fn copy_whitelisted_headers(source: &HeaderMap, target: &mut HeaderMap, names: &[String]) {
    for raw in names {
        if let Ok(name) = HeaderName::from_bytes(raw.as_bytes())
            && forwardable_header(&name)
        {
            let values = source.get_all(&name);
            if values.iter().next().is_some() {
                target.remove(&name);
                for value in values {
                    target.append(&name, value.clone());
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Secret([u8; 32]);
impl Secret {
    pub fn generate() -> Result<Self> {
        let mut key = [0; 32];
        OsRng
            .try_fill_bytes(&mut key)
            .context("Operating-system entropy unavailable")?;
        Ok(Self(key))
    }
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match private_create(path) {
            Ok(mut file) => {
                let secret = Self::generate()?;
                file.write_all(&secret.0)?;
                file.sync_all()?;
                Ok(secret)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let meta = std::fs::symlink_metadata(path)?;
                ensure!(
                    meta.file_type().is_file() && !meta.file_type().is_symlink(),
                    "Secret must be a regular file, not a symlink"
                );
                let mut bytes = Vec::new();
                File::open(path)?.take(33).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() == 32,
                    "Secret is corrupt; restore it rather than silently invalidating all sessions"
                );
                let mut key = [0; 32];
                key.copy_from_slice(&bytes);
                Ok(Self(key))
            }
            Err(e) => Err(e.into()),
        }
    }
    pub fn mac(&self, context: &str, data: &str) -> String {
        let mut h = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts a 32-byte key");
        h.update(context.as_bytes());
        h.update(&[0]);
        h.update(data.as_bytes());
        URL_SAFE_NO_PAD.encode(h.finalize().into_bytes())
    }
    pub fn sign(&self, context: &str, value: &str) -> String {
        let payload = format!("{}:{}", epoch_seconds(), value);
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload.as_bytes()),
            self.mac(context, &payload)
        )
    }
    pub fn verify(&self, context: &str, token: &str, lifetime: u64) -> Option<String> {
        if token.len() > 1024 {
            return None;
        }
        let (encoded, signature) = token.split_once('.')?;
        let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()?;
        if !constant_eq(signature, &self.mac(context, &payload)) {
            return None;
        }
        let (stamp, value) = payload.split_once(':')?;
        let stamp: u64 = stamp.parse().ok()?;
        let now = epoch_seconds();
        if stamp > now.saturating_add(30) || now.saturating_sub(stamp) > lifetime {
            return None;
        }
        Some(value.to_owned())
    }
    pub fn visitor(&self, ip: IpAddr) -> String {
        // Daily rotation prevents persistent cross-day linkage in the database.
        self.mac("visitor", &format!("{}:{}", epoch_seconds() / 86400, ip))
    }
}
pub fn private_create(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}
pub fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "State directory cannot be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn random_id() -> String {
    let mut bytes = [0; 16];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn constant_eq(a: &str, b: &str) -> bool {
    // Hashing gives constant-length operands and avoids a token-length oracle.
    bool::from(Sha256::digest(a.as_bytes()).ct_eq(&Sha256::digest(b.as_bytes())))
}
pub fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for h in headers.get_all(header::COOKIE) {
        for part in h.to_str().ok()?.split(';') {
            if let Some((key, value)) = part.trim().split_once('=')
                && key == name
            {
                if found.is_some() {
                    return None;
                }
                found = Some(value.to_owned());
            }
        }
    }
    found
}
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let peer = peer.to_canonical();
    if !trusted.iter().any(|net| net.contains(&peer)) {
        return peer;
    }
    let mut chain = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(s) = value.to_str() else {
            return peer;
        };
        for part in s.split(',') {
            let Ok(ip) = part.trim().parse::<IpAddr>() else {
                return peer;
            };
            chain.push(ip.to_canonical());
            if chain.len() > 32 {
                return peer;
            }
        }
    }
    let mut current = peer;
    for ip in chain.into_iter().rev() {
        if !trusted.iter().any(|net| net.contains(&current)) {
            break;
        }
        current = ip;
    }
    current
}
pub fn strip_hop_headers(headers: &mut HeaderMap, upgrading: bool) {
    let connection_names: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|n| n.trim().parse::<HeaderName>().ok())
        .collect();
    for name in connection_names {
        if !upgrading || name != header::UPGRADE {
            headers.remove(name);
        }
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "transfer-encoding",
    ] {
        headers.remove(name);
    }
    if upgrading {
        headers.insert(
            header::CONNECTION,
            http::HeaderValue::from_static("upgrade"),
        );
    } else {
        headers.remove(header::UPGRADE);
    }
    // Trailer is intentionally preserved: streamed bodies keep their trailer frames.
}
pub fn is_websocket(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && headers
            .get_all(header::CONNECTION)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|s| s.split(','))
            .any(|s| s.trim().eq_ignore_ascii_case("upgrade"))
}
pub fn referrer_origin(headers: &HeaderMap) -> String {
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| url::Url::parse(s).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_tokens_are_context_bound_and_tamper_evident() {
        let s = Secret::generate().unwrap();
        let t = s.sign("app1", "worker1");
        assert_eq!(s.verify("app1", &t, 60).as_deref(), Some("worker1"));
        assert!(s.verify("app2", &t, 60).is_none());
        assert!(s.verify("app1", &(t + "x"), 60).is_none());
    }
    #[test]
    fn forwarded_ips_require_a_trusted_peer() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 10.0.0.2".parse().unwrap());
        let nets = ["10.0.0.0/8".parse().unwrap()];
        assert_eq!(
            client_ip("8.8.8.8".parse().unwrap(), &h, &nets).to_string(),
            "8.8.8.8"
        );
        assert_eq!(
            client_ip("10.0.0.1".parse().unwrap(), &h, &nets).to_string(),
            "1.2.3.4"
        );
        h.insert("x-forwarded-for", "1.2.3.4, 9.9.9.9".parse().unwrap());
        assert_eq!(
            client_ip("10.0.0.1".parse().unwrap(), &h, &nets).to_string(),
            "9.9.9.9"
        );
    }
    #[test]
    fn connection_nominated_headers_are_removed() {
        let mut h = HeaderMap::new();
        h.insert("connection", "x-private, keep-alive".parse().unwrap());
        h.insert("x-private", "secret".parse().unwrap());
        strip_hop_headers(&mut h, false);
        assert!(!h.contains_key("x-private"));
        assert!(!h.contains_key("connection"));
    }
    #[test]
    fn duplicate_cookies_are_rejected() {
        let mut h = HeaderMap::new();
        h.append("cookie", "id=a; id=b".parse().unwrap());
        assert!(cookie(&h, "id").is_none());
    }
    #[test]
    fn referrers_never_store_paths_or_queries() {
        let mut h = HeaderMap::new();
        h.insert(
            "referer",
            "https://example.org/private?q=secret".parse().unwrap(),
        );
        assert_eq!(referrer_origin(&h), "https://example.org");
    }
}
