//! Authenticated configuration editing on the separate, loopback admin listener.
use crate::{config::Config, reload::Control, security, server};
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use http::{HeaderMap, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::Path, sync::Arc};

const MAX_CONFIG: usize = 1024 * 1024;
fn revision(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}
fn response(value: Value) -> Response {
    let mut result = Json(value).into_response();
    result.headers_mut().insert(
        header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    result.headers_mut().insert(
        "x-content-type-options",
        http::HeaderValue::from_static("nosniff"),
    );
    result
}
fn error(status: StatusCode, message: &str) -> Response {
    let mut result = response(json!({"error":message}));
    *result.status_mut() = status;
    result
}
fn source(path: &Path) -> Result<String> {
    ensure!(
        std::fs::metadata(path)?.len() <= MAX_CONFIG as u64,
        "Configuration is larger than the editor's 1 MiB limit"
    );
    Ok(std::fs::read_to_string(path)?)
}
pub(crate) async fn get(State(control): State<Arc<Control>>, headers: HeaderMap) -> Response {
    let state = control.snapshot();
    if !server::authorized(&headers, &state) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Unlock administration with your admin token.",
        );
    }
    let Some(path) = &control.path else {
        return error(
            StatusCode::CONFLICT,
            "This server was started without an editable configuration file.",
        );
    };
    match source(path) {
        Ok(text) => {
            let pending = Config::from_text(&text, path)
                .ok()
                .and_then(|c| serde_json::to_value(c).ok())
                != serde_json::to_value(&*state.config).ok();
            response(json!({"source":text,"revision":revision(&text),"path":path,
                "format":if path.extension().is_some_and(|e| e == "conf") {"legacy"} else {"toml"},
                "active":&*state.config,"configured":Config::from_text(&text, path).ok(),"defaults":Config::default(),"pending":pending}))
        }
        Err(e) => error(
            StatusCode::BAD_REQUEST,
            &format!("Cannot read configuration: {e:#}"),
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    revision: String,
    source: Option<String>,
    #[serde(default)]
    changes: BTreeMap<String, Value>,
    #[serde(default)]
    save: bool,
}
fn same_origin(headers: &HeaderMap) -> bool {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    }; // CLI has no Origin.
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    origin
        .to_str()
        .is_ok_and(|origin| origin == format!("http://{host}"))
}
fn patched(text: &str, changes: &BTreeMap<String, Value>) -> Result<String> {
    let mut document: toml_edit::DocumentMut = text.parse()?;
    for (path, value) in changes {
        let keys: Vec<_> = path.split('.').collect();
        ensure!(
            !keys.is_empty()
                && keys.len() <= 4
                && keys
                    .iter()
                    .all(|k| !k.is_empty()
                        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')),
            "Invalid setting name"
        );
        // The guided editor handles scalar settings and small tables. The source
        // editor handles arrays of apps/servers and preserves legacy syntax.
        let encoded = toml::Value::try_from(value).context("Unsupported setting value")?;
        let wrapper = toml::to_string(&BTreeMap::from([("value", encoded)]))?;
        let parsed: toml_edit::DocumentMut = wrapper.parse()?;
        let mut cursor = document.as_item_mut();
        for key in &keys[..keys.len() - 1] {
            if cursor.get(*key).is_none() {
                cursor[*key] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            ensure!(
                cursor[*key].is_table(),
                "This setting requires the full configuration editor"
            );
            cursor = &mut cursor[*key];
        }
        cursor[keys[keys.len() - 1]] = parsed["value"].clone();
    }
    Ok(document.to_string())
}
async fn validate_tls(config: &Config) -> Result<()> {
    for address in config.listeners() {
        if let crate::config::Tls::Pem { cert, key } = config.listener_tls(address) {
            axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .context("The HTTPS certificate/private key could not be loaded")?;
        }
    }
    Ok(())
}
fn save(path: &Path, old: &str, text: &str) -> Result<std::path::PathBuf> {
    let parent = path.parent().context("Configuration has no parent")?;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Cannot replace a symbolic-link configuration"
    );
    let mut draft = tempfile::Builder::new().prefix(".shinyharbor-").tempfile_in(parent)
        .context("Cannot write beside the configuration. Grant the service account access to its configuration directory, or edit the file locally")?;
    draft.write_all(text.as_bytes())?;
    draft
        .as_file()
        .set_permissions(std::fs::metadata(path)?.permissions())?;
    draft.as_file().sync_all()?;
    let directory = parent.join(".shinyharbor-backups");
    security::private_dir(&directory)?;
    let backup = directory.join(format!(
        "{}-{}.bak",
        security::epoch_seconds(),
        security::random_id()
    ));
    let mut file = security::private_create(&backup)?;
    file.write_all(old.as_bytes())?;
    file.sync_all()?;
    ensure!(
        source(path)? == old,
        "Configuration changed on disk during save. Refresh and retry"
    );
    draft
        .persist(path)
        .context("Could not replace the configuration; the original and backup were retained")?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(backup)
}
pub(crate) async fn edit(
    State(control): State<Arc<Control>>,
    headers: HeaderMap,
    Json(edit): Json<Edit>,
) -> Response {
    if !server::authorized(&headers, &control.snapshot()) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Unlock administration with your admin token.",
        );
    }
    if !same_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "Configuration edits must come from this administration page.",
        );
    }
    let _exclusive = control.reloading.lock().await;
    let Some(path) = &control.path else {
        return error(StatusCode::CONFLICT, "No configuration file to edit.");
    };
    let old = match source(path) {
        Ok(text) => text,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if edit.revision != revision(&old) {
        return error(
            StatusCode::CONFLICT,
            "The file changed since you opened it. Reload settings before saving; your draft has not been applied.",
        );
    }
    let text = match edit.source {
        Some(text) if edit.changes.is_empty() => text,
        None if path.extension().is_none_or(|e| e != "conf") => {
            match patched(&old, &edit.changes) {
                Ok(text) => text,
                Err(e) => {
                    return error(
                        StatusCode::BAD_REQUEST,
                        &format!("Cannot prepare settings: {e:#}"),
                    );
                }
            }
        }
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "Use either the guided controls or the source editor. Legacy .conf files use the source editor.",
            );
        }
    };
    if text.len() > MAX_CONFIG {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Configuration must be at most 1 MiB.",
        );
    }
    let config = match Config::from_text(&text, path) {
        Ok(config) => config,
        Err(e) => {
            return error(
                StatusCode::BAD_REQUEST,
                &format!("Configuration was not saved: {e:#}"),
            );
        }
    };
    if let Err(e) = validate_tls(&config).await {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("Configuration was not saved: {e:#}"),
        );
    }
    let current = control.snapshot();
    let restart = crate::reload::fixed_settings(&config).ok()
        != crate::reload::fixed_settings(&current.config).ok();
    let mut result = json!({"valid":true,"source":text,"revision":revision(&text),"restart_required":restart,"saved":false,
        "message":if restart {"Valid. Saving these global settings requires a server restart to activate them. Existing listeners will keep running until then."} else {"Valid. These changes can be applied without disconnecting existing sessions."}});
    if edit.save {
        let backup = match save(path, &old, &text) {
            Ok(path) => path,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("Save failed: {e:#}")),
        };
        result["saved"] = json!(true);
        result["backup"] = json!(backup);
        if !restart {
            if let Err(e) = control.apply(config).await {
                result["restart_required"] = json!(true);
                result["message"] = json!(format!(
                    "Saved with a backup, but applying failed: {e:#}. Restart after reviewing the file."
                ));
            } else {
                result["message"] = json!("Saved and applied. Existing sessions remain connected.");
            }
        } else {
            result["message"] = json!(
                "Saved with a backup. Restart ShinyHarbor to activate these settings; the current listeners are still running."
            );
        }
    }
    response(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_patch_preserves_comments_and_other_settings() {
        let input =
            "# my configuration\nlisten = '127.0.0.1:3838' # local only\n[tls]\nmode='off'\n";
        let result = patched(
            input,
            &BTreeMap::from([("analytics.retention_days".into(), json!(60))]),
        )
        .unwrap();
        assert!(result.contains("# local only"));
        assert_eq!(
            toml::from_str::<Config>(&result)
                .unwrap()
                .analytics
                .retention_days,
            60
        );
        assert!(
            patched(
                input,
                &BTreeMap::from([("tls.mode.invalid".into(), json!(true))])
            )
            .is_err()
        );
    }
    #[test]
    fn cross_origin_writes_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:3839".parse().unwrap());
        headers.insert(header::ORIGIN, "https://unrelated.example".parse().unwrap());
        assert!(!same_origin(&headers));
        headers.insert(header::ORIGIN, "http://localhost:3839".parse().unwrap());
        assert!(same_origin(&headers));
    }
}
