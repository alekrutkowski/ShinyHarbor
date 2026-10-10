use crate::security::{Secret, epoch_seconds};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use base64::{Engine, engine::general_purpose::STANDARD};
use http::{HeaderMap, StatusCode};
use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;

pub struct Auth {
    proxy: Option<crate::config::ProxyAuth>,
    users: Arc<BTreeMap<String, String>>,
    secret: Secret,
    cache: Mutex<HashMap<String, (String, u64)>>,
    work: Arc<Semaphore>,
}
impl Auth {
    pub fn new(users: BTreeMap<String, String>, secret: Secret) -> Self {
        Self {
            proxy: None,
            users: Arc::new(users),
            secret,
            cache: Mutex::new(HashMap::new()),
            work: Arc::new(Semaphore::new(4)),
        }
    }
    pub fn with_proxy(mut self, proxy: Option<crate::config::ProxyAuth>) -> Self {
        self.proxy = proxy;
        self
    }
    pub fn basic_challenge(&self) -> bool {
        self.proxy.is_none()
    }
    pub fn strip_identity_headers(&self, headers: &mut HeaderMap) {
        if let Some(proxy) = &self.proxy {
            headers.remove(&proxy.user_header);
            if let Some(groups) = &proxy.groups_header {
                headers.remove(groups);
            }
        }
    }
    pub async fn authorize_request(
        &self,
        headers: &HeaderMap,
        peer: IpAddr,
        users: &[String],
        groups: &[String],
        ignore_case: bool,
    ) -> Result<Option<Identity>, StatusCode> {
        let identity = if let Some(proxy) = &self.proxy {
            if proxy.trusted_peers.iter().any(|net| net.contains(&peer)) {
                let user = single_header(headers, &proxy.user_header)?;
                let group_list = match &proxy.groups_header {
                    Some(name) => single_header(headers, name)?,
                    None => None,
                };
                match user {
                    Some(user) if valid_identity(user) => {
                        let mut groups = Vec::new();
                        if let Some(raw) = group_list.filter(|s| !s.is_empty()) {
                            for group in raw.split(',').map(str::trim) {
                                if !valid_identity(group) || groups.len() >= 256 {
                                    return Err(StatusCode::UNAUTHORIZED);
                                }
                                groups.push(group.to_owned());
                            }
                        }
                        groups.sort();
                        groups.dedup();
                        Some(Identity {
                            user: user.to_owned(),
                            groups,
                        })
                    }
                    Some(_) => return Err(StatusCode::UNAUTHORIZED),
                    None if group_list.is_some() => return Err(StatusCode::UNAUTHORIZED),
                    None => None,
                }
            } else {
                None
            }
        } else {
            let required = if users.is_empty() || users.iter().any(|u| u == "**") {
                vec![]
            } else {
                vec!["*".to_owned()]
            };
            self.authorize(headers, &required)
                .await?
                .map(|user| Identity {
                    user,
                    groups: vec![],
                })
        };
        let public = users.is_empty() || users.iter().any(|u| u == "**");
        let user_allowed = public
            || identity.as_ref().is_some_and(|i| {
                users
                    .iter()
                    .any(|u| u == "*" || identity_matches(u, &i.user, ignore_case))
            });
        let group_allowed = groups.is_empty()
            || identity.as_ref().is_some_and(|i| {
                groups
                    .iter()
                    .any(|g| i.groups.iter().any(|v| identity_matches(g, v, ignore_case)))
            });
        if user_allowed && group_allowed {
            Ok(identity)
        } else {
            Err(if identity.is_some() {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            })
        }
    }
    async fn authorize(
        &self,
        headers: &HeaderMap,
        allowed: &[String],
    ) -> Result<Option<String>, StatusCode> {
        if allowed.is_empty() || allowed.iter().any(|u| u == "**") {
            return Ok(None);
        }
        let raw = single_header(headers, "authorization")?.ok_or(StatusCode::UNAUTHORIZED)?;
        if raw.len() > 4096 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let key = self.secret.mac("auth-cache", raw);
        {
            let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((user, expiry)) = cache.get(&key)
                && *expiry >= epoch_seconds()
            {
                return if allowed.contains(user) || allowed.iter().any(|u| u == "*") {
                    Ok(Some(user.clone()))
                } else {
                    Err(StatusCode::FORBIDDEN)
                };
            }
        }
        let encoded = raw.strip_prefix("Basic ").ok_or(StatusCode::UNAUTHORIZED)?;
        let decoded = String::from_utf8(
            STANDARD
                .decode(encoded)
                .map_err(|_| StatusCode::UNAUTHORIZED)?,
        )
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let (name, password) = decoded.split_once(':').ok_or(StatusCode::UNAUTHORIZED)?;
        let name = name.to_owned();
        let password = password.to_owned();
        let hash = self
            .users
            .get(&name)
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let permit = self
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            PasswordHash::new(&hash).is_ok_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !valid {
            return Err(StatusCode::UNAUTHORIZED);
        }
        {
            let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.retain(|_, (_, expiry)| *expiry >= epoch_seconds());
            if cache.len() >= 512 {
                cache.clear();
            }
            cache.insert(key, (name.clone(), epoch_seconds() + 30));
        }
        if allowed.contains(&name) || allowed.iter().any(|u| u == "*") {
            Ok(Some(name))
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

// A transport session is bound to both the authenticated name and group set.
// A reconnect cannot retain privileges after the proxy changes that identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Identity {
    pub user: String,
    pub groups: Vec<String>,
}
pub fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
pub fn identity_matches(left: &str, right: &str, ignore_case: bool) -> bool {
    left == right || (ignore_case && left.to_lowercase() == right.to_lowercase())
}
fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|v| v.to_str().map_err(|_| StatusCode::UNAUTHORIZED))
        .transpose()?;
    if values.next().is_some() || value.is_some_and(|v| v.len() > 8192) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{PasswordHasher, password_hash::SaltString};
    use http::header;
    #[tokio::test]
    async fn proxy_identity_trust_groups_and_ambiguous_headers() {
        let auth = Auth::new(BTreeMap::new(), Secret::generate().unwrap()).with_proxy(Some(
            crate::config::ProxyAuth {
                groups_header: Some("x-groups".into()),
                ..Default::default()
            },
        ));
        let peer = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-auth-username", "alice".parse().unwrap());
        headers.insert("x-groups", "users, admins, users".parse().unwrap());
        let expected = Some(Identity {
            user: "alice".into(),
            groups: vec!["admins".into(), "users".into()],
        });
        assert_eq!(
            auth.authorize_request(&headers, peer, &["*".into()], &["admins".into()], false)
                .await,
            Ok(expected.clone())
        );
        assert_eq!(
            auth.authorize_request(&headers, peer, &[], &[], false)
                .await,
            Ok(expected)
        );
        assert_eq!(
            auth.authorize_request(&headers, peer, &["bob".into()], &["admins".into()], false)
                .await,
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            auth.authorize_request(&headers, peer, &["alice".into()], &["other".into()], false)
                .await,
            Err(StatusCode::FORBIDDEN)
        );
        headers.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert_eq!(
            auth.authorize_request(
                &headers,
                "192.0.2.4".parse().unwrap(),
                &["alice".into()],
                &[],
                false
            )
            .await,
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.append("x-auth-username", "bob".parse().unwrap());
        assert_eq!(
            auth.authorize_request(&headers, peer, &["*".into()], &[], false)
                .await,
            Err(StatusCode::UNAUTHORIZED)
        );
        auth.strip_identity_headers(&mut headers);
        assert!(!headers.contains_key("x-auth-username") && !headers.contains_key("x-groups"));
        assert_eq!(
            auth.authorize_request(&headers, peer, &["**".into()], &[], false)
                .await,
            Ok(None)
        );
    }
    #[tokio::test]
    async fn credentials_and_route_allowlists_are_both_enforced() {
        let salt = SaltString::encode_b64(b"test-salt-unique").unwrap();
        let hash = Argon2::default()
            .hash_password(b"test-password", &salt)
            .unwrap()
            .to_string();
        let auth = Auth::new(
            BTreeMap::from([("alice".into(), hash)]),
            Secret::generate().unwrap(),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "shiny-server-credentials",
            "{\"user\":\"alice\"}".parse().unwrap(),
        );
        assert_eq!(
            auth.authorize(&headers, &["alice".into()]).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {}", STANDARD.encode("alice:test-password"))
                .parse()
                .unwrap(),
        );
        assert_eq!(
            auth.authorize(&headers, &["alice".into()]).await,
            Ok(Some("alice".into()))
        );
        // A cached valid credential is still checked against each route's policy.
        assert_eq!(
            auth.authorize(&headers, &["bob".into()]).await,
            Err(StatusCode::FORBIDDEN)
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {}", STANDARD.encode("alice:wrong"))
                .parse()
                .unwrap(),
        );
        assert_eq!(
            auth.authorize(&headers, &["alice".into()]).await,
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(auth.authorize(&headers, &[]).await, Ok(None));
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {}", STANDARD.encode("alice:test-password"))
                .parse()
                .unwrap(),
        );
        let peer = "127.0.0.1".parse().unwrap();
        assert_eq!(
            auth.authorize_request(&headers, peer, &["ALICE".into()], &[], true)
                .await
                .unwrap()
                .unwrap()
                .user,
            "alice"
        );
        assert_eq!(
            auth.authorize_request(&headers, peer, &["ALICE".into()], &[], false)
                .await,
            Err(StatusCode::FORBIDDEN)
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {}", STANDARD.encode("ALICE:test-password"))
                .parse()
                .unwrap(),
        );
        assert_eq!(
            auth.authorize_request(&headers, peer, &["alice".into()], &[], true)
                .await,
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.append(header::AUTHORIZATION, "Basic junk".parse().unwrap());
        assert_eq!(
            auth.authorize(&headers, &["*".into()]).await,
            Err(StatusCode::UNAUTHORIZED)
        );
    }
}
