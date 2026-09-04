//! Unified API key store: local (`sk-aria_`) and oauth (`sk-bf-`) in one `router-keys.json`.

use aria_router_core::RouterError;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

const SECRET_PREFIX: &str = "sk-aria_";
/// Prefix that identifies an Aria Compute (serve) OAuth API key pasted into the
/// router. Serve issues keys as `sk-bf-…`.
const OAUTH_KEY_PREFIXES: &[&str] = &["sk-bf-"];

fn is_oauth_key(secret: &str) -> bool {
    OAUTH_KEY_PREFIXES.iter().any(|p| secret.starts_with(p))
}

const LINK_STATE_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    #[default]
    Local,
    Oauth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServeUserInfo {
    pub id: serde_json::Value,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

/// Inputs for linking a serve (Aria Compute) account to the router. Bundled into
/// a single struct so `KeyStore::apply_exchange` stays within clippy's
/// argument-count limit.
pub struct ExchangeInput {
    pub site: String,
    pub site_url: String,
    pub user: ServeUserInfo,
    pub link_token: Option<String>,
    pub expires_at: Option<String>,
    pub api_key: Option<(String, String)>,
    pub owner_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRecord {
    #[serde(default)]
    pub kind: KeyKind,
    pub id: String,
    pub name: String,
    pub prefix: String,
    #[serde(default)]
    pub secret_sha256: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub owner_user_id: Option<String>,
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
    /// Set when a sync detects the stored serve API key is gone (deleted or
    /// revoked) on serve. The stale secret is cleared, but name/prefix are kept
    /// for display so the dashboard can show which key was removed.
    #[serde(default)]
    pub api_key_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeyFile {
    #[serde(default)]
    keys: Vec<KeyRecord>,
}

#[derive(Debug, Clone)]
struct PendingLink {
    site: String,
    site_url: String,
    expires_unix: u64,
    owner_user_id: Option<String>,
    owner_email: Option<String>,
}

#[derive(Debug)]
pub struct KeyStore {
    path: PathBuf,
    keys: Vec<KeyRecord>,
    pending: Mutex<HashMap<String, PendingLink>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyPublic {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyCreated {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub secret: String,
    pub created_at: String,
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
    /// The stored serve API key was deleted/revoked on serve. The router keeps
    /// the last-known name/prefix for display and clears the stale secret.
    pub api_key_deleted: bool,
    pub status: String,
}

#[derive(Debug, Clone)]
pub enum AuthIdentity {
    Local {
        id: String,
        name: String,
        owner_user_id: Option<String>,
    },
    Oauth {
        id: String,
        name: Option<String>,
        email: Option<String>,
        site: Option<String>,
        user_id: Option<String>,
    },
}

impl KeyStore {
    pub fn load(path: &Path) -> Result<Self, RouterError> {
        let keys = if path.exists() {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                RouterError::Io(format!("read {}: {e}", path.display()))
            })?;
            let file: KeyFile = serde_json::from_str(&raw).map_err(|e| {
                RouterError::Config(format!("keys file {}: {e}", path.display()))
            })?;
            file.keys
        } else {
            Vec::new()
        };
        let mut store = Self {
            path: path.to_path_buf(),
            keys,
            pending: Mutex::new(HashMap::new()),
        };
        store.migrate_legacy_serve()?;
        Ok(store)
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            keys: Vec::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn legacy_serve_path(keys_path: &Path) -> PathBuf {
        keys_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("router-serve.json")
    }

    fn migrate_legacy_serve(&mut self) -> Result<(), RouterError> {
        if self.active_oauth().is_some() {
            return Ok(());
        }
        let legacy = Self::legacy_serve_path(&self.path);
        if !legacy.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&legacy).map_err(|e| {
            RouterError::Io(format!("read {}: {e}", legacy.display()))
        })?;
        #[derive(Deserialize)]
        struct LegacyServe {
            #[serde(default)]
            site: Option<String>,
            #[serde(default)]
            site_url: Option<String>,
            #[serde(default)]
            gateway_url: Option<String>,
            #[serde(default)]
            user: Option<ServeUserInfo>,
            #[serde(default)]
            linked_at: Option<String>,
            #[serde(default)]
            link_expires_at: Option<String>,
            #[serde(default)]
            link_token: Option<String>,
            #[serde(default)]
            api_key_name: Option<String>,
            #[serde(default)]
            api_key_prefix: Option<String>,
            #[serde(default)]
            api_key: Option<String>,
        }
        let data: LegacyServe = serde_json::from_str(&raw).map_err(|e| {
            RouterError::Config(format!("legacy serve {}: {e}", legacy.display()))
        })?;
        let has_anything = data.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false)
            || data.user.is_some()
            || data.site.is_some();
        if has_anything {
            let api_key = data.api_key.clone().unwrap_or_default();
            let prefix = data.api_key_prefix.clone().unwrap_or_else(|| {
                api_key.chars().take(16).collect()
            });
            let rec = KeyRecord {
                kind: KeyKind::Oauth,
                id: format!("oauth_{}", hex::encode({
                    let mut r = [0u8; 8];
                    rand::thread_rng().fill_bytes(&mut r);
                    r
                })),
                name: data.api_key_name.unwrap_or_else(|| "aria-router".into()),
                prefix,
                secret_sha256: String::new(),
                api_key: if api_key.is_empty() { None } else { Some(api_key) },
                created_at: data.linked_at.clone().unwrap_or_else(now_rfc3339),
                last_used_at: None,
                revoked: false,
                owner_user_id: None,
                site: data.site,
                site_url: data.site_url,
                gateway_url: data.gateway_url,
                user: data.user,
                linked_at: data.linked_at,
                link_expires_at: data.link_expires_at,
                link_token: data.link_token,
                api_key_deleted: false,
            };
            self.keys.push(rec);
            self.persist()?;
        }
        let _ = std::fs::remove_file(&legacy);
        Ok(())
    }

    fn persist(&self) -> Result<(), RouterError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RouterError::Io(format!("mkdir {}: {e}", parent.display()))
            })?;
        }
        let file = KeyFile {
            keys: self.keys.clone(),
        };
        let raw = serde_json::to_string_pretty(&file).map_err(|e| {
            RouterError::Io(e.to_string())
        })?;
        std::fs::write(&self.path, raw).map_err(|e| {
            RouterError::Io(format!("write {}: {e}", self.path.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn is_local(k: &KeyRecord) -> bool {
        matches!(k.kind, KeyKind::Local)
    }

    fn is_oauth(k: &KeyRecord) -> bool {
        matches!(k.kind, KeyKind::Oauth)
    }

    pub fn list_public(&self) -> Vec<KeyPublic> {
        self.keys
            .iter()
            .filter(|k| Self::is_local(k))
            .map(|k| KeyPublic {
                id: k.id.clone(),
                name: k.name.clone(),
                prefix: k.prefix.clone(),
                created_at: k.created_at.clone(),
                last_used_at: k.last_used_at.clone(),
                revoked: k.revoked,
                owner_user_id: k.owner_user_id.clone(),
            })
            .collect()
    }

    pub fn list_for_owner(&self, owner_id: &str, is_admin: bool) -> Vec<KeyPublic> {
        self.list_public()
            .into_iter()
            .filter(|k| is_admin || k.owner_user_id.as_deref() == Some(owner_id))
            .collect()
    }

    pub fn counts(&self) -> (usize, usize) {
        let local: Vec<_> = self.keys.iter().filter(|k| Self::is_local(k)).collect();
        let active = local.iter().filter(|k| !k.revoked).count();
        let revoked = local.iter().filter(|k| k.revoked).count();
        (active, revoked)
    }

    pub fn create(&mut self, name: &str) -> Result<KeyCreated, RouterError> {
        self.create_for(name, None)
    }

    pub fn create_for(
        &mut self,
        name: &str,
        owner_user_id: Option<String>,
    ) -> Result<KeyCreated, RouterError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RouterError::InvalidParam("key name required".into()));
        }
        let mut rnd = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut rnd);
        let secret = format!("{SECRET_PREFIX}{}", hex::encode(rnd));
        let hash = sha256_hex(&secret);
        let id = format!("key_{}", hex::encode(&rnd[..8]));
        let created_at = now_rfc3339();
        let prefix: String = secret.chars().take(12).collect();
        let rec = KeyRecord {
            kind: KeyKind::Local,
            id: id.clone(),
            name: name.to_string(),
            prefix: prefix.clone(),
            secret_sha256: hash,
            api_key: None,
            created_at: created_at.clone(),
            last_used_at: None,
            revoked: false,
            owner_user_id,
            site: None,
            site_url: None,
            gateway_url: None,
            user: None,
            linked_at: None,
            link_expires_at: None,
            link_token: None,
            api_key_deleted: false,
        };
        self.keys.push(rec);
        self.persist()?;
        Ok(KeyCreated {
            id,
            name: name.to_string(),
            prefix,
            secret,
            created_at,
        })
    }

    /// Authenticate any router API key (local sha256 or oauth plaintext).
    pub fn authenticate(
        &mut self,
        secret: &str,
    ) -> Result<(String, String, Option<String>), RouterError> {
        match self.resolve_bearer(secret)? {
            AuthIdentity::Local {
                id,
                name,
                owner_user_id,
            } => Ok((id, name, owner_user_id)),
            AuthIdentity::Oauth { id, name, .. } => Ok((id, name.unwrap_or_default(), None)),
        }
    }

    pub fn resolve_bearer(&mut self, secret: &str) -> Result<AuthIdentity, RouterError> {
        let secret = secret.trim();
        if is_oauth_key(secret) {
            return self.authenticate_oauth(secret);
        }
        let hash = sha256_hex(secret);
        let Some(k) = self
            .keys
            .iter_mut()
            .find(|k| Self::is_local(k) && k.secret_sha256 == hash)
        else {
            return Err(RouterError::Unauthorized("invalid api key".into()));
        };
        if k.revoked {
            return Err(RouterError::Unauthorized("api key revoked".into()));
        }
        k.last_used_at = Some(now_rfc3339());
        let id = k.id.clone();
        let name = k.name.clone();
        let owner = k.owner_user_id.clone();
        let _ = self.persist();
        Ok(AuthIdentity::Local {
            id,
            name,
            owner_user_id: owner,
        })
    }

    fn authenticate_oauth(&mut self, secret: &str) -> Result<AuthIdentity, RouterError> {
        let Some(idx) = self.keys.iter().position(|k| {
            Self::is_oauth(k)
                && !k.revoked
                && k.api_key.as_deref().map(|s| {
                    let a = secret.as_bytes();
                    let b = s.as_bytes();
                    a.len() == b.len() && bool::from(a.ct_eq(b))
                })
                .unwrap_or(false)
        }) else {
            return Err(RouterError::Unauthorized(
                "OAuth key not linked; configure Dashboard Account or aria-router setup".into(),
            ));
        };
        let k = &mut self.keys[idx];
        k.last_used_at = Some(now_rfc3339());
        let id = format!("serve:{}", k.prefix);
        let name = Some(k.name.clone());
        let email = k
            .user
            .as_ref()
            .and_then(|u| u.email.clone())
            .or_else(|| Some(id.clone()));
        let site = k.site.clone();
        let user_id = k.user.as_ref().map(|u| u.id.to_string());
        let _ = self.persist();
        Ok(AuthIdentity::Oauth {
            id,
            name,
            email,
            site,
            user_id,
        })
    }

    pub fn owner_of(&self, id: &str) -> Option<Option<String>> {
        self.keys
            .iter()
            .find(|k| k.id == id)
            .map(|k| k.owner_user_id.clone())
    }

    pub fn revoke(&mut self, id: &str) -> Result<(), RouterError> {
        let Some(k) = self.keys.iter_mut().find(|k| k.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown key id {id}")));
        };
        k.revoked = true;
        self.persist()?;
        Ok(())
    }

    fn active_oauth(&self) -> Option<&KeyRecord> {
        self.keys
            .iter()
            .find(|k| Self::is_oauth(k) && !k.revoked)
    }

    fn active_oauth_mut(&mut self) -> Option<&mut KeyRecord> {
        self.keys
            .iter_mut()
            .find(|k| Self::is_oauth(k) && !k.revoked)
    }

    fn revoke_active_oauth(&mut self) {
        for k in &mut self.keys {
            if Self::is_oauth(k) && !k.revoked {
                k.revoked = true;
            }
        }
    }

    pub fn oauth_public(&self) -> ServeAccountPublic {
        match self.active_oauth() {
            None => ServeAccountPublic {
                linked: false,
                site: None,
                site_url: None,
                gateway_url: None,
                user: None,
                linked_at: None,
                api_key_name: None,
                api_key_prefix: None,
                api_key_configured: false,
                api_key_deleted: false,
                status: "not linked".into(),
            },
            Some(k) => {
                let linked = k.user.is_some();
                let configured = k.api_key.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
                let deleted = k.api_key_deleted;
                let status = if deleted {
                    "api key deleted on serve".into()
                } else if linked && configured {
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
                    site: k.site.clone(),
                    site_url: k.site_url.clone(),
                    gateway_url: k.gateway_url.clone(),
                    user: k.user.clone(),
                    linked_at: k.linked_at.clone(),
                    api_key_name: Some(k.name.clone()),
                    api_key_prefix: Some(k.prefix.clone()),
                    api_key_configured: configured,
                    api_key_deleted: deleted,
                    status,
                }
            }
        }
    }

    pub fn oauth_reveal_secret(&self) -> Option<String> {
        self.active_oauth()
            .and_then(|k| k.api_key.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn oauth_clear(&mut self) -> Result<(), RouterError> {
        self.revoke_active_oauth();
        self.persist()
    }

    pub fn oauth_set_api_key(
        &mut self,
        api_key: &str,
        name: Option<&str>,
    ) -> Result<(), RouterError> {
        let key = api_key.trim();
        validate_oauth_key(key)?;
        let prefix: String = key.chars().take(16).collect();
        let name = name.unwrap_or("aria-router").to_string();
        if let Some(k) = self.active_oauth_mut() {
            k.api_key = Some(key.to_string());
            k.prefix = prefix;
            k.name = name;
            // A freshly pasted key is valid again; clear any prior deletion mark.
            k.api_key_deleted = false;
        } else {
            self.keys.push(KeyRecord {
                kind: KeyKind::Oauth,
                id: format!("oauth_{}", {
                    let mut r = [0u8; 8];
                    rand::thread_rng().fill_bytes(&mut r);
                    hex::encode(r)
                }),
                name,
                prefix,
                secret_sha256: String::new(),
                api_key: Some(key.to_string()),
                created_at: now_rfc3339(),
                last_used_at: None,
                revoked: false,
                owner_user_id: None,
                site: None,
                site_url: None,
                gateway_url: None,
                user: None,
                linked_at: None,
                link_expires_at: None,
                link_token: None,
                api_key_deleted: false,
            });
        }
        self.persist()
    }

    /// Mark the stored serve API key as deleted/revoked on serve: clear the
    /// stale secret so the router stops authenticating with it, but keep the
    /// name/prefix for display. Set when a sync detects the key is gone (a 401
    /// on listing keys, or its prefix no longer in the serve key list).
    pub fn oauth_mark_api_key_deleted(&mut self) -> Result<(), RouterError> {
        let k = self
            .active_oauth_mut()
            .ok_or_else(|| RouterError::InvalidParam("no linked serve account".into()))?;
        k.api_key_deleted = true;
        k.api_key = None;
        self.persist()
    }

    /// Clear the deleted marker (e.g. after the user pastes a fresh, valid key
    /// or a sync re-finds the key on serve).
    pub fn oauth_unmark_api_key_deleted(&mut self) -> Result<(), RouterError> {
        let k = self
            .active_oauth_mut()
            .ok_or_else(|| RouterError::InvalidParam("no linked serve account".into()))?;
        k.api_key_deleted = false;
        self.persist()
    }

    #[cfg(test)]
    pub(crate) fn set_site_url_for_test(&mut self, url: &str) {
        if let Some(k) = self.active_oauth_mut() {
            k.site_url = Some(url.to_string());
        }
    }

    pub fn oauth_set_site(&mut self, site: &str) -> Result<(), RouterError> {
        let (site, site_url, gateway_url) = normalize_site(site)?;
        if let Some(k) = self.active_oauth_mut() {
            k.site = Some(site);
            k.site_url = Some(site_url);
            k.gateway_url = Some(gateway_url);
        } else {
            self.keys.push(KeyRecord {
                kind: KeyKind::Oauth,
                id: format!("oauth_{}", {
                    let mut r = [0u8; 8];
                    rand::thread_rng().fill_bytes(&mut r);
                    hex::encode(r)
                }),
                name: "aria-router".into(),
                prefix: String::new(),
                secret_sha256: String::new(),
                api_key: None,
                created_at: now_rfc3339(),
                last_used_at: None,
                revoked: false,
                owner_user_id: None,
                site: Some(site),
                site_url: Some(site_url),
                gateway_url: Some(gateway_url),
                user: None,
                linked_at: None,
                link_expires_at: None,
                link_token: None,
                api_key_deleted: false,
            });
        }
        self.persist()
    }

    fn pct_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    pub fn begin_link(
        &self,
        site: &str,
        owner_user_id: Option<String>,
        owner_email: Option<String>,
        owner_name: Option<String>,
    ) -> Result<(String, String, String), RouterError> {
        let (site_id, site_url, _gw) = normalize_site(site)?;
        let mut rnd = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut rnd);
        let state = hex::encode(rnd);
        let expires_unix = now_unix() + LINK_STATE_TTL_SECS;
        let email_part = match owner_email.as_deref() {
            Some(e) if !e.is_empty() => format!("&email={}", Self::pct_encode(e)),
            _ => String::new(),
        };
        let name_part = match owner_name.as_deref() {
            Some(n) if !n.is_empty() => format!("&name={}", Self::pct_encode(n)),
            _ => String::new(),
        };
        self.pending.lock().unwrap().insert(
            state.clone(),
            PendingLink {
                site: site_id,
                site_url: site_url.clone(),
                expires_unix,
                owner_user_id,
                owner_email,
            },
        );
        let authorize_url = format!(
            "{}/api/router-link/start?callback={{callback}}&state={state}{email_part}{name_part}",
            site_url.trim_end_matches('/')
        );
        Ok((authorize_url, state, site_url))
    }

    pub fn take_pending(
        &self,
        state: &str,
    ) -> Result<(String, String, Option<String>, Option<String>), RouterError> {
        let mut map = self.pending.lock().unwrap();
        let Some(p) = map.remove(state) else {
            return Err(RouterError::Unauthorized("invalid oauth state".into()));
        };
        if p.expires_unix < now_unix() {
            return Err(RouterError::Unauthorized("oauth state expired".into()));
        }
        Ok((p.site, p.site_url, p.owner_user_id, p.owner_email))
    }

pub fn apply_exchange(&mut self, inp: ExchangeInput) -> Result<(), RouterError> {
    let ExchangeInput {
        site,
        site_url,
        user,
        link_token,
        expires_at,
        api_key,
        owner_user_id,
    } = inp;
    let (site_id, site_url_n, gateway_url) = match normalize_site(&site) {
        Ok(v) => v,
        Err(_) => (
            if site.contains("cn") {
                "cn".into()
            } else {
                "intl".into()
            },
            site_url,
            gateway_for_site(&site),
        ),
    };
        if self.active_oauth().is_none() {
            self.keys.push(KeyRecord {
                kind: KeyKind::Oauth,
                id: format!("oauth_{}", {
                    let mut r = [0u8; 8];
                    rand::thread_rng().fill_bytes(&mut r);
                    hex::encode(r)
                }),
                name: "aria-router".into(),
                prefix: String::new(),
                secret_sha256: String::new(),
                api_key: None,
                created_at: now_rfc3339(),
                last_used_at: None,
                revoked: false,
                owner_user_id: None,
                site: None,
                site_url: None,
                gateway_url: None,
                user: None,
                linked_at: None,
                link_expires_at: None,
                link_token: None,
                api_key_deleted: false,
            });
        }
        let k = self.active_oauth_mut().expect("oauth row");
        k.site = Some(site_id);
        k.site_url = Some(site_url_n);
        k.gateway_url = Some(gateway_url);
        k.user = Some(user);
        k.linked_at = Some(now_rfc3339());
        k.link_token = link_token;
        k.link_expires_at = expires_at;
        k.owner_user_id = owner_user_id;
        if let Some((name, key)) = api_key {
            validate_oauth_key(&key)?;
            let prefix: String = key.chars().take(16).collect();
            k.api_key = Some(key);
            k.prefix = prefix;
            k.name = name;
        }
        self.persist()
    }

    pub fn oauth_site_url(&self) -> Option<String> {
        self.active_oauth().and_then(|k| k.site_url.clone())
    }

    pub fn oauth_link_token(&self) -> Option<String> {
        self.active_oauth().and_then(|k| k.link_token.clone())
    }

    /// The stored serve API key (`sk-bf-`), if configured. Used as a durable
    /// credential to call back into serve (e.g. to sync key metadata) after the
    /// short-lived `link_token` has expired.
    pub fn oauth_api_key(&self) -> Option<String> {
        self.active_oauth()
            .and_then(|k| k.api_key.clone())
            .filter(|s| !s.is_empty())
    }

    /// The serve owner user id of the linked account, if known. Used to decide
    /// whether a re-link targets the same account (so its existing serve key can
    /// be reused) versus a different account (which needs a fresh key).
    pub fn oauth_owner_user_id(&self) -> Option<String> {
        self.active_oauth().and_then(|k| k.owner_user_id.clone())
    }

    /// Update the displayed serve API key name/prefix without touching the
    /// stored secret. Used by the serve account sync flow.
    pub fn oauth_set_api_key_meta(
        &mut self,
        name: String,
        prefix: String,
    ) -> Result<(), RouterError> {
        let k = self
            .active_oauth_mut()
            .ok_or_else(|| RouterError::InvalidParam("no linked serve account".into()))?;
        k.name = name;
        k.prefix = prefix;
        self.persist()
    }

    /// Clear the stored serve API key (sk-bf-) without unlinking the account. Used
    /// when re-linking a *different* serve account so the previous account's key is
    /// not reused for the new one.
    pub fn oauth_clear_api_key(&mut self) -> Result<(), RouterError> {
        let k = self
            .active_oauth_mut()
            .ok_or_else(|| RouterError::InvalidParam("no linked serve account".into()))?;
        k.api_key = None;
        k.prefix = String::new();
        self.persist()
    }
}

pub fn validate_oauth_key(key: &str) -> Result<(), RouterError> {
    if key.starts_with("sk-aria_") {
        return Err(RouterError::InvalidParam(
            "Local router key detected; use Dashboard → Keys".into(),
        ));
    }
    if !is_oauth_key(key) {
        return Err(RouterError::InvalidParam(
            "OAuth API key must start with sk-bf-".into(),
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

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
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

pub fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            let s = s.trim();
            if let Some(rest) = s.strip_prefix("Bearer ") {
                let t = rest.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    if let Some(v) = headers.get("x-api-key") {
        if let Ok(s) = v.to_str() {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_auth_revoke() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path);
        let created = store.create("ci").unwrap();
        assert!(created.secret.starts_with(SECRET_PREFIX));
        let (id, name, _) = store.authenticate(&created.secret).unwrap();
        assert_eq!(id, created.id);
        assert_eq!(name, "ci");
        store.revoke(&created.id).unwrap();
        assert!(store.authenticate(&created.secret).is_err());
    }

    #[test]
    fn oauth_serve_key_roundtrip_and_migrate() {
        let dir = tempdir().unwrap();
        let keys_path = dir.path().join("router-keys.json");
        let legacy = dir.path().join("router-serve.json");
        std::fs::write(
            &legacy,
            r#"{"site":"com","site_url":"https://ariacompute.com","api_key":"sk-bf-abcdefghijklmnop","api_key_prefix":"sk-bf-abcdefgh","api_key_name":"test"}"#,
        )
        .unwrap();
        let mut store = KeyStore::load(&keys_path).unwrap();
        assert!(!legacy.exists());
        assert!(store.oauth_public().api_key_configured);
        assert!(matches!(
            store.resolve_bearer("sk-bf-abcdefghijklmnop").unwrap(),
            AuthIdentity::Oauth { .. }
        ));
        assert!(store.resolve_bearer("sk-bf-wrong").is_err());
        assert!(validate_oauth_key("sk-aria_abc").is_err());
    }

    #[test]
    fn serve_api_key_accepts_sk_bf_prefix() {
        // Serve issues OAuth API keys with the `sk-bf-` prefix.
        assert!(validate_oauth_key("sk-bf-59af311a-803d-41c0-8000-b82ee4d46b7c").is_ok());
        assert!(is_oauth_key("sk-bf-59af311a-803d-41c0-8000-b82ee4d46b7c"));
        assert!(!is_oauth_key("sk-aria_abc"));
        // A `sk-bf-` key routes to OAuth (not local) authentication.
        let dir = tempdir().unwrap();
        let keys_path = dir.path().join("router-keys.json");
        let mut store = KeyStore::load(&keys_path).unwrap();
        store
            .oauth_set_api_key("sk-bf-59af311a-803d-41c0-8000-b82ee4d46b7c", Some("test"))
            .unwrap();
        assert!(matches!(
            store
                .resolve_bearer("sk-bf-59af311a-803d-41c0-8000-b82ee4d46b7c")
                .unwrap(),
            AuthIdentity::Oauth { .. }
        ));
    }

    #[test]
    fn oauth_link_carries_owner_identity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let store = KeyStore::empty(path);
        let (_, state, _) = store
            .begin_link(
                "com",
                Some("user-123".into()),
                Some("jia@ariacompute.com".into()),
                None,
            )
            .unwrap();
        let (site, _site_url, owner_id, owner_email) = store.take_pending(&state).unwrap();
        assert_eq!(site, "intl");
        assert_eq!(owner_id.as_deref(), Some("user-123"));
        assert_eq!(owner_email.as_deref(), Some("jia@ariacompute.com"));
        // Consumed once; second take must fail.
        assert!(store.take_pending(&state).is_err());
    }

    #[test]
    fn oauth_set_api_key_meta_updates_display_without_secret() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path.clone());
        let (_, state, _) = store
            .begin_link("com", Some("user-1".into()), Some("a@b.com".into()), None)
            .unwrap();
        let _ = store.take_pending(&state).unwrap();
        store
            .apply_exchange(ExchangeInput {
                site: "https://ariacompute.com".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::String("serve-1".into()),
                    email: Some("a@b.com".into()),
                    role: Some("user".into()),
                },
                link_token: Some("lt".into()),
                expires_at: None,
                api_key: None,
                owner_user_id: Some("user-1".into()),
            })
            .unwrap();

        let before = store.oauth_public();
        assert!(!before.api_key_configured);
        assert_eq!(before.api_key_name.as_deref(), Some("aria-router"));

        // Sync metadata from serve without touching the stored secret.
        store
            .oauth_set_api_key_meta("aria-router-pro".into(), "sk-bf-ABCD1234".into())
            .unwrap();
        let after = store.oauth_public();
        assert_eq!(after.api_key_name.as_deref(), Some("aria-router-pro"));
        assert_eq!(after.api_key_prefix.as_deref(), Some("sk-bf-ABCD1234"));
        // Secret was never set, so still not configured.
        assert!(!after.api_key_configured);
        assert!(store.oauth_reveal_secret().is_none());
    }

    #[test]
    fn oauth_set_api_key_meta_requires_link() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path);
        assert!(store.oauth_set_api_key_meta("x".into(), "y".into()).is_err());
    }

    #[test]
    fn oauth_owner_user_id_tracks_linked_account() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path.clone());
        assert!(store.oauth_owner_user_id().is_none());
        let (_, state, _) = store
            .begin_link("com", Some("user-1".into()), Some("a@b.com".into()), None)
            .unwrap();
        let _ = store.take_pending(&state).unwrap();
        store
            .apply_exchange(ExchangeInput {
                site: "https://ariacompute.com".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::String("serve-1".into()),
                    email: Some("a@b.com".into()),
                    role: Some("user".into()),
                },
                link_token: Some("lt".into()),
                expires_at: None,
                api_key: Some(("aria-router".into(), "sk-bf-ABCD1234EFGH5678".into())),
                owner_user_id: Some("user-1".into()),
            })
            .unwrap();
        // Linked with a key on the same account -> owner recorded.
        assert_eq!(store.oauth_owner_user_id().as_deref(), Some("user-1"));
        assert!(store.oauth_api_key().is_some());
    }

    #[test]
    fn oauth_mark_api_key_deleted_clears_secret_and_flags() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path.clone());
        let (_, state, _) = store
            .begin_link("com", Some("user-1".into()), Some("a@b.com".into()), None)
            .unwrap();
        let _ = store.take_pending(&state).unwrap();
        store
            .apply_exchange(ExchangeInput {
                site: "https://ariacompute.com".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::String("serve-1".into()),
                    email: Some("a@b.com".into()),
                    role: Some("user".into()),
                },
                link_token: Some("lt".into()),
                expires_at: None,
                api_key: Some(("aria-router".into(), "sk-bf-ABCDEFGHIJKLMNOP".into())),
                owner_user_id: Some("user-1".into()),
            })
            .unwrap();
        assert!(store.oauth_public().api_key_configured);
        assert!(!store.oauth_public().api_key_deleted);

        // Marking deleted clears the stale secret but keeps name/prefix.
        store.oauth_mark_api_key_deleted().unwrap();
        let acct = store.oauth_public();
        assert!(acct.api_key_deleted);
        assert!(!acct.api_key_configured);
        assert_eq!(acct.api_key_prefix.as_deref(), Some("sk-bf-ABCDEFGHIJ"));
        assert!(store.oauth_api_key().is_none());

        // Re-pasting a (fresh) key clears the deletion marker.
        store
            .oauth_set_api_key("sk-bf-NEWKEY1234567890", Some("k2"))
            .unwrap();
        let acct = store.oauth_public();
        assert!(!acct.api_key_deleted);
        assert!(acct.api_key_configured);
    }
}
