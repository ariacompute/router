//! OAuth (Aria Compute) account link + bfvk storage for aria-router.

use aria_router_core::RouterError;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

const BFVK_PREFIX: &str = "bfvk-";
const LINK_STATE_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServeAccountFile {
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub site_url: Option<String>,
    #[serde(default)]
    pub gateway_url: Option<String>,
    #[serde(default)]
    pub user: Option<ServeUserInfo>,
    #[serde(default)]
    pub linked_at: Option<String>,
    #[serde(default)]
    pub link_expires_at: Option<String>,
    #[serde(default)]
    pub link_token: Option<String>,
    #[serde(default)]
    pub api_key_name: Option<String>,
    #[serde(default)]
    pub api_key_prefix: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeUserInfo {
    pub id: serde_json::Value,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeAccountPublic {
    pub linked: bool,
    pub site: Option<String>,
    pub site_url: Option<String>,
    pub gateway_url: Option<String>,
    pub user: Option<ServeUserInfo>,
    pub linked_at: Option<String>,
    pub api_key_name: Option<String>,
    pub api_key_prefix: Option<String>,
    pub api_key_configured: bool,
    pub status: String,
}

#[derive(Debug, Clone)]
struct PendingLink {
    site: String,
    site_url: String,
    expires_unix: u64,
}

#[derive(Debug)]
pub struct ServeAccountStore {
    path: PathBuf,
    data: ServeAccountFile,
    pending: Mutex<HashMap<String, PendingLink>>,
}

impl ServeAccountStore {
    pub fn load(path: &Path) -> Result<Self, RouterError> {
        let data = if path.exists() {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                RouterError::Io(format!("read {}: {e}", path.display()))
            })?;
            serde_json::from_str(&raw).map_err(|e| {
                RouterError::Config(format!("serve account {}: {e}", path.display()))
            })?
        } else {
            ServeAccountFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            data: ServeAccountFile::default(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn persist(&self) -> Result<(), RouterError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RouterError::Io(format!("mkdir {}: {e}", parent.display()))
            })?;
        }
        let raw = serde_json::to_string_pretty(&self.data).map_err(|e| RouterError::Io(e.to_string()))?;
        std::fs::write(&self.path, raw).map_err(|e| {
            RouterError::Io(format!("write {}: {e}", self.path.display()))
        })?;
        // Best-effort restrict permissions on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn public(&self) -> ServeAccountPublic {
        let linked = self.data.user.is_some();
        let configured = self
            .data
            .api_key
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        let status = if linked && configured {
            "linked".into()
        } else if configured {
            "key only, not linked".into()
        } else if linked {
            "linked, no api key".into()
        } else {
            "not linked".into()
        };
        ServeAccountPublic {
            linked,
            site: self.data.site.clone(),
            site_url: self.data.site_url.clone(),
            gateway_url: self.data.gateway_url.clone(),
            user: self.data.user.clone(),
            linked_at: self.data.linked_at.clone(),
            api_key_name: self.data.api_key_name.clone(),
            api_key_prefix: self.data.api_key_prefix.clone(),
            api_key_configured: configured,
            status,
        }
    }

    pub fn reveal_secret(&self) -> Option<String> {
        self.data.api_key.clone().filter(|k| !k.is_empty())
    }

    pub fn clear(&mut self) -> Result<(), RouterError> {
        self.data = ServeAccountFile::default();
        if self.path.exists() {
            std::fs::remove_file(&self.path).map_err(|e| {
                RouterError::Io(format!("remove {}: {e}", self.path.display()))
            })?;
        }
        Ok(())
    }

    pub fn set_api_key(&mut self, api_key: &str, name: Option<&str>) -> Result<(), RouterError> {
        let key = api_key.trim();
        validate_bfvk(key)?;
        let prefix: String = key.chars().take(16).collect();
        self.data.api_key = Some(key.to_string());
        self.data.api_key_prefix = Some(prefix);
        self.data.api_key_name = Some(name.unwrap_or("aria-router").to_string());
        if self.data.site.is_none() {
            // leave site unset until link/site chosen
        }
        self.persist()
    }

    pub fn set_site(&mut self, site: &str) -> Result<(), RouterError> {
        let (site, site_url, gateway_url) = normalize_site(site)?;
        self.data.site = Some(site);
        self.data.site_url = Some(site_url);
        self.data.gateway_url = Some(gateway_url);
        self.persist()
    }

    pub fn begin_link(&self, site: &str) -> Result<(String, String, String), RouterError> {
        let (site_id, site_url, _gw) = normalize_site(site)?;
        let mut rnd = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut rnd);
        let state = hex::encode(rnd);
        let expires_unix = now_unix() + LINK_STATE_TTL_SECS;
        self.pending.lock().unwrap().insert(
            state.clone(),
            PendingLink {
                site: site_id.clone(),
                site_url: site_url.clone(),
                expires_unix,
            },
        );
        let authorize_url = format!(
            "{}/api/router-link/start?callback={{callback}}&state={state}",
            site_url.trim_end_matches('/')
        );
        Ok((authorize_url, state, site_url))
    }

    pub fn take_pending(&self, state: &str) -> Result<(String, String), RouterError> {
        let mut map = self.pending.lock().unwrap();
        let Some(p) = map.remove(state) else {
            return Err(RouterError::Unauthorized("invalid oauth state".into()));
        };
        if p.expires_unix < now_unix() {
            return Err(RouterError::Unauthorized("oauth state expired".into()));
        }
        Ok((p.site, p.site_url))
    }

    pub fn apply_exchange(
        &mut self,
        site: &str,
        site_url: &str,
        user: ServeUserInfo,
        link_token: Option<String>,
        expires_at: Option<String>,
        api_key: Option<(String, String)>,
    ) -> Result<(), RouterError> {
        let (site_id, site_url_n, gateway_url) = match normalize_site(site) {
            Ok(v) => v,
            Err(_) => (
                if site.contains("cn") {
                    "cn".into()
                } else {
                    "intl".into()
                },
                site_url.to_string(),
                gateway_for_site(site),
            ),
        };
        self.data.site = Some(site_id);
        self.data.site_url = Some(site_url_n);
        self.data.gateway_url = Some(gateway_url);
        self.data.user = Some(user);
        self.data.linked_at = Some(now_rfc3339());
        self.data.link_token = link_token;
        self.data.link_expires_at = expires_at;
        if let Some((name, key)) = api_key {
            validate_bfvk(&key)?;
            let prefix: String = key.chars().take(16).collect();
            self.data.api_key = Some(key);
            self.data.api_key_prefix = Some(prefix);
            self.data.api_key_name = Some(name);
        }
        self.persist()
    }

    pub fn authenticate_bfvk(&self, secret: &str) -> Option<(String, Option<String>)> {
        let stored = self.data.api_key.as_deref()?;
        let a = secret.trim().as_bytes();
        let b = stored.as_bytes();
        if a.len() != b.len() || !bool::from(a.ct_eq(b)) {
            return None;
        }
        let prefix = self
            .data
            .api_key_prefix
            .clone()
            .unwrap_or_else(|| secret.chars().take(16).collect());
        let email = self
            .data
            .user
            .as_ref()
            .and_then(|u| u.email.clone())
            .or_else(|| Some(format!("serve:{prefix}")));
        Some((format!("serve:{prefix}"), email))
    }

    pub fn site_url(&self) -> Option<String> {
        self.data.site_url.clone()
    }

    pub fn link_token(&self) -> Option<String> {
        self.data.link_token.clone()
    }
}

pub fn validate_bfvk(key: &str) -> Result<(), RouterError> {
    if key.starts_with("sk-aria_") {
        return Err(RouterError::InvalidParam(
            "Local router key detected; use [1/2] Local / Dashboard → Keys".into(),
        ));
    }
    if !key.starts_with(BFVK_PREFIX) {
        return Err(RouterError::InvalidParam(
            "OAuth API key must start with bfvk-".into(),
        ));
    }
    if key.len() < 12 {
        return Err(RouterError::InvalidParam("OAuth API key too short".into()));
    }
    Ok(())
}

pub fn normalize_site(site: &str) -> Result<(String, String, String), RouterError> {
    let s = site.trim().to_ascii_lowercase();
    match s.as_str() {
        "intl" | "com" | "1" | "https://ariacompute.com" | "ariacompute.com" => Ok((
            "intl".into(),
            "https://ariacompute.com".into(),
            "https://gateway.ariacompute.com".into(),
        )),
        "cn" | "2" | "https://ariacompute.cn" | "ariacompute.cn" => Ok((
            "cn".into(),
            "https://ariacompute.cn".into(),
            "https://gateway.ariacompute.cn".into(),
        )),
        other if other.contains("ariacompute.cn") => Ok((
            "cn".into(),
            "https://ariacompute.cn".into(),
            "https://gateway.ariacompute.cn".into(),
        )),
        other if other.contains("ariacompute.com") => Ok((
            "intl".into(),
            "https://ariacompute.com".into(),
            "https://gateway.ariacompute.com".into(),
        )),
        _ => Err(RouterError::InvalidParam(
            "serve site must be com|cn (ariacompute.com / ariacompute.cn)".into(),
        )),
    }
}

fn gateway_for_site(site: &str) -> String {
    if site.contains("cn") {
        "https://gateway.ariacompute.cn".into()
    } else {
        "https://gateway.ariacompute.com".into()
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bfvk_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("serve.json");
        let mut store = ServeAccountStore::empty(path);
        store.set_site("com").unwrap();
        store.set_api_key("bfvk-abcdefghijklmnop", Some("test")).unwrap();
        let pubu = store.public();
        assert!(pubu.api_key_configured);
        assert!(store.authenticate_bfvk("bfvk-abcdefghijklmnop").is_some());
        assert!(store.authenticate_bfvk("bfvk-wrong").is_none());
        assert!(validate_bfvk("sk-aria_abc").is_err());
    }
}
