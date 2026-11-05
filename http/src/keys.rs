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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// Opt-in spend quota (C7). `None` = unlimited. All amounts are USD
    /// estimates derived from forwarded traffic, never provider balances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_limit_usd: Option<f64>,
    /// Keys sharing a group share one cumulative spend counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_group: Option<String>,
    /// `once` (default) or `calendar_month`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_period: Option<String>,
    /// Persistent accumulator — independent of the 4096-event cost ring.
    #[serde(default)]
    pub spend_used_usd: f64,
    /// Calendar bucket the accumulator belongs to (YYYY-MM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_period_key: Option<String>,
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
            let raw = std::fs::read_to_string(path)
                .map_err(|e| RouterError::Io(format!("read {}: {e}", path.display())))?;
            let file: KeyFile = serde_json::from_str(&raw)
                .map_err(|e| RouterError::Config(format!("keys file {}: {e}", path.display())))?;
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
        let raw = std::fs::read_to_string(&legacy)
            .map_err(|e| RouterError::Io(format!("read {}: {e}", legacy.display())))?;
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
        let data: LegacyServe = serde_json::from_str(&raw)
            .map_err(|e| RouterError::Config(format!("legacy serve {}: {e}", legacy.display())))?;
        let has_anything = data
            .api_key
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false)
            || data.user.is_some()
            || data.site.is_some();
        if has_anything {
            let api_key = data.api_key.clone().unwrap_or_default();
            let prefix = data
                .api_key_prefix
                .clone()
                .unwrap_or_else(|| api_key.chars().take(16).collect());
            let rec = KeyRecord {
                kind: KeyKind::Oauth,
                id: format!(
                    "oauth_{}",
                    hex::encode({
                        let mut r = [0u8; 8];
                        rand::thread_rng().fill_bytes(&mut r);
                        r
                    })
                ),
                name: data.api_key_name.unwrap_or_else(|| "aria-router".into()),
                prefix,
                secret_sha256: String::new(),
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(api_key)
                },
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
                ..Default::default()
            };
            self.keys.push(rec);
            self.persist()?;
        }
        let _ = std::fs::remove_file(&legacy);
        Ok(())
    }

    fn persist(&self) -> Result<(), RouterError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RouterError::Io(format!("mkdir {}: {e}", parent.display())))?;
        }
        let file = KeyFile {
            keys: self.keys.clone(),
        };
        let raw =
            serde_json::to_string_pretty(&file).map_err(|e| RouterError::Io(e.to_string()))?;
        std::fs::write(&self.path, raw)
            .map_err(|e| RouterError::Io(format!("write {}: {e}", self.path.display())))?;
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
            ..Default::default()
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
                && k.api_key
                    .as_deref()
                    .map(|s| {
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

    /// Configure an opt-in spend quota for one key (C7). `None` removes the
    /// limit but keeps the recorded spend. Grouped keys share one cumulative
    /// counter; calendar-month periods roll over at the month boundary.
    pub fn set_spend_policy(
        &mut self,
        id: &str,
        limit: Option<f64>,
        group: Option<String>,
        period: Option<String>,
    ) -> Result<(), RouterError> {
        if limit.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err(RouterError::InvalidParam(
                "spend_limit_usd must be a finite non-negative number".into(),
            ));
        }
        if period
            .as_deref()
            .is_some_and(|p| p != "once" && p != "calendar_month")
        {
            return Err(RouterError::InvalidParam(
                "spend_period must be once|calendar_month".into(),
            ));
        }
        let Some(k) = self.keys.iter_mut().find(|k| k.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown key id {id}")));
        };
        k.spend_limit_usd = limit;
        k.spend_group = group.filter(|g| !g.trim().is_empty());
        k.spend_period = period;
        self.persist()?;
        Ok(())
    }

    fn roll_calendar(&mut self, id: &str, period_key: &str) -> Result<(), RouterError> {
        let group = self
            .keys
            .iter()
            .find(|k| k.id == id)
            .and_then(|k| k.spend_group.clone());
        let in_scope = |k: &KeyRecord| match group.as_deref() {
            Some(g) => k.spend_group.as_deref() == Some(g),
            None => k.id == id,
        };
        let needs_roll = self.keys.iter().any(|k| {
            k.spend_period.as_deref() == Some("calendar_month")
                && k.spend_period_key.as_deref() != Some(period_key)
                && in_scope(k)
        });
        if !needs_roll {
            return Ok(());
        }
        for k in &mut self.keys {
            if k.spend_period.as_deref() == Some("calendar_month") && in_scope(k) {
                k.spend_used_usd = 0.0;
                k.spend_period_key = Some(period_key.to_string());
            }
        }
        self.persist()
    }

    fn spend_group_limit_and_used(&self, id: &str) -> Result<(Option<f64>, f64), RouterError> {
        let key = self
            .keys
            .iter()
            .find(|k| k.id == id)
            .ok_or_else(|| RouterError::InvalidParam(format!("unknown key id {id}")))?;
        let group = key.spend_group.clone();
        let members: Vec<&KeyRecord> = self
            .keys
            .iter()
            .filter(|k| {
                k.id == id
                    || group
                        .as_deref()
                        .is_some_and(|g| k.spend_group.as_deref() == Some(g))
            })
            .collect();
        let limit = members
            .iter()
            .filter_map(|k| k.spend_limit_usd)
            .reduce(f64::min);
        let used: f64 = members.iter().map(|k| k.spend_used_usd).sum();
        Ok((limit, used))
    }

    /// Remaining allowance before a reservation may be made. `None` means the
    /// key (or its group) has no configured limit.
    pub fn spend_remaining(
        &mut self,
        id: &str,
        period_key: &str,
        pending_usd: f64,
    ) -> Result<Option<f64>, RouterError> {
        self.roll_calendar(id, period_key)?;
        let (limit, used) = self.spend_group_limit_and_used(id)?;
        Ok(limit.map(|l| l - used - pending_usd))
    }

    /// Persist settled spend. Settlement uses the actual estimate, so an
    /// interrupted stream only consumes the traffic that actually happened.
    pub fn record_spend(
        &mut self,
        id: &str,
        usd: f64,
        period_key: &str,
    ) -> Result<(), RouterError> {
        self.roll_calendar(id, period_key)?;
        let Some(k) = self.keys.iter_mut().find(|k| k.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown key id {id}")));
        };
        k.spend_used_usd += usd.max(0.0);
        self.persist()
    }

    pub fn spend_view(&self, id: &str, period_key: &str) -> Result<serde_json::Value, RouterError> {
        let (limit, used) = self.spend_group_limit_and_used(id)?;
        let k = self.keys.iter().find(|k| k.id == id).unwrap();
        Ok(serde_json::json!({
            "key_id": id,
            "limit_usd": limit,
            "used_usd": used,
            "remaining_usd": limit.map(|l| l - used),
            "group": k.spend_group,
            "period": k.spend_period,
            "period_key": if k.spend_period.as_deref() == Some("calendar_month") { Some(period_key) } else { None },
        }))
    }

    fn active_oauth(&self) -> Option<&KeyRecord> {
        self.keys.iter().find(|k| Self::is_oauth(k) && !k.revoked)
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
                ..Default::default()
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

    /// Register a pending OAuth link that points at an arbitrary serve URL, so
    /// the callback exchange can be driven against a local mock server.
    #[cfg(test)]
    pub(crate) fn begin_link_at_for_test(&self, site_url: &str) -> String {
        let mut rnd = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut rnd);
        let state = hex::encode(rnd);
        self.pending.lock().unwrap().insert(
            state.clone(),
            PendingLink {
                site: "intl".into(),
                site_url: site_url.to_string(),
                expires_unix: now_unix() + LINK_STATE_TTL_SECS,
                owner_user_id: None,
                owner_email: None,
            },
        );
        state
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
                ..Default::default()
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
                ..Default::default()
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
            // `s` is already trimmed and the prefix ends with a space, so a
            // non-empty `rest` is guaranteed here — no empty-token check needed.
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return Some(rest.trim().to_string());
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
    fn persist_without_parent_directory_reports_io_error() {
        // A root path has no parent, so `persist` skips mkdir and fails on write.
        let mut store = KeyStore::empty(std::path::PathBuf::from("/"));
        let err = store.create("rootless").unwrap_err();
        assert!(matches!(err, RouterError::Io(_)), "{err}");
    }

    #[test]
    fn spend_quota_reserve_settle_and_shared_group() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = KeyStore::empty(path.clone());
        let a = store.create("a").unwrap();
        let b = store.create("b").unwrap();
        assert_eq!(store.spend_remaining(&a.id, "2026-11", 0.0).unwrap(), None);
        store
            .set_spend_policy(&a.id, Some(1.0), Some("team".into()), Some("once".into()))
            .unwrap();
        store
            .set_spend_policy(&b.id, Some(1.0), Some("team".into()), Some("once".into()))
            .unwrap();
        assert_eq!(
            store.spend_remaining(&a.id, "2026-11", 0.3).unwrap(),
            Some(0.7)
        );
        store.record_spend(&a.id, 0.4, "2026-11").unwrap();
        // Both keys share the cumulative counter.
        assert_eq!(
            store.spend_remaining(&b.id, "2026-11", 0.1).unwrap(),
            Some(0.5)
        );
        store.record_spend(&b.id, 0.6, "2026-11").unwrap();
        assert_eq!(
            store.spend_remaining(&a.id, "2026-11", 0.0).unwrap(),
            Some(0.0)
        );
        // Persistent accumulator survives a reload (not the 4096-event ring).
        let reloaded = KeyStore::load(&path).unwrap();
        assert_eq!(
            reloaded.spend_view(&a.id, "2026-11").unwrap()["used_usd"],
            1.0
        );
    }

    #[test]
    fn spend_quota_calendar_rollover_and_validation_errors() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let a = store.create("a").unwrap();
        store
            .set_spend_policy(&a.id, Some(2.0), None, Some("calendar_month".into()))
            .unwrap();
        store.record_spend(&a.id, 1.5, "2026-10").unwrap();
        assert_eq!(
            store.spend_remaining(&a.id, "2026-10", 0.0).unwrap(),
            Some(0.5)
        );
        assert_eq!(
            store.spend_remaining(&a.id, "2026-11", 0.0).unwrap(),
            Some(2.0)
        );
        assert!(matches!(
            store.set_spend_policy(&a.id, Some(-1.0), None, None),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            store.set_spend_policy(&a.id, None, None, Some("weekly".into())),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            store.set_spend_policy("missing", Some(1.0), None, None),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            store.record_spend("missing", 1.0, "2026-11"),
            Err(RouterError::InvalidParam(_))
        ));
        assert!(matches!(
            store.spend_remaining("missing", "2026-11", 0.0),
            Err(RouterError::InvalidParam(_))
        ));
        // Removing the limit keeps accounting but never blocks.
        store
            .set_spend_policy(&a.id, None, None, Some("once".into()))
            .unwrap();
        assert_eq!(store.spend_remaining(&a.id, "2026-11", 0.0).unwrap(), None);
        let view = store.spend_view(&a.id, "2026-11").unwrap();
        assert!(view["limit_usd"].is_null());
        assert!(view["period_key"].is_null());
    }

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
        assert!(store
            .oauth_set_api_key_meta("x".into(), "y".into())
            .is_err());
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

    fn linked_store(dir: &tempfile::TempDir) -> KeyStore {
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let (_, state, _) = store.begin_link("com", None, None, None).unwrap();
        let _ = store.take_pending(&state).unwrap();
        store
            .apply_exchange(ExchangeInput {
                site: "https://ariacompute.com".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::String("serve-1".into()),
                    email: Some("a@b.com".into()),
                    role: None,
                },
                link_token: Some("lt".into()),
                expires_at: None,
                api_key: None,
                owner_user_id: None,
            })
            .unwrap();
        store
    }

    #[test]
    fn list_public_and_owner_scoping() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let a = store.create_for("mine", Some("usr_1".into())).unwrap();
        let b = store.create("theirs").unwrap();
        assert_eq!(store.counts(), (2, 0));
        let mine = store.list_public();
        assert_eq!(mine.len(), 2);
        assert_eq!(
            mine.iter()
                .find(|k| k.id == a.id)
                .and_then(|k| k.owner_user_id.clone())
                .as_deref(),
            Some("usr_1")
        );
        // Owner scoping: non-admins only see their own keys.
        assert_eq!(store.list_for_owner("usr_1", false).len(), 1);
        assert_eq!(store.list_for_owner("usr_1", true).len(), 2);
        assert!(store.list_for_owner("usr_2", false).is_empty());
        // last_used_at is stamped on successful auth.
        store.authenticate(&a.secret).unwrap();
        let used = store
            .list_public()
            .into_iter()
            .find(|k| k.id == a.id)
            .unwrap();
        assert!(used.last_used_at.is_some());

        // Revoking an unknown key fails loudly; counts track revoked keys.
        let err = store.revoke("key_nope").unwrap_err();
        assert!(err.to_string().contains("unknown key id"), "{err}");
        store.revoke(&b.id).unwrap();
        assert_eq!(store.counts(), (1, 1));
    }

    #[test]
    fn create_requires_name_and_owner_lookup() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let err = store.create("   ").unwrap_err();
        assert!(err.to_string().contains("key name required"), "{err}");
        let created = store.create_for("owned", Some("usr_9".into())).unwrap();
        assert_eq!(
            store.owner_of(&created.id).flatten().as_deref(),
            Some("usr_9")
        );
        assert!(store.owner_of("key_missing").is_none());
        let anonymous = store.create("anon").unwrap();
        assert_eq!(store.owner_of(&anonymous.id), Some(None));
    }

    #[test]
    fn authenticate_local_unknown_and_oauth_identity() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let err = store.authenticate("sk-aria_unknown").unwrap_err();
        assert!(err.to_string().contains("invalid api key"), "{err}");
        // An OAuth key with no linked account cannot authenticate.
        let err = store.authenticate("sk-bf-not-linked").unwrap_err();
        assert!(err.to_string().contains("OAuth key not linked"), "{err}");

        store
            .oauth_set_api_key("sk-bf-REVEAL12345678", Some("oauth-name"))
            .unwrap();
        let (id, name, owner) = store.authenticate("sk-bf-REVEAL12345678").unwrap();
        assert!(id.starts_with("serve:"), "{id}");
        assert_eq!(name, "oauth-name");
        assert!(owner.is_none());
        assert_eq!(
            store.oauth_reveal_secret().as_deref(),
            Some("sk-bf-REVEAL12345678")
        );
        assert_eq!(store.oauth_site_url(), None);
        assert_eq!(store.oauth_link_token(), None);
    }

    #[test]
    fn oauth_unlink_clear_and_site_updates() {
        let dir = tempdir().unwrap();
        let mut store = linked_store(&dir);
        assert_eq!(
            store.oauth_site_url().as_deref(),
            Some("https://ariacompute.com")
        );
        assert_eq!(store.oauth_link_token().as_deref(), Some("lt"));

        // Switching to the CN site rewrites site/site_url/gateway_url in place.
        store.oauth_set_site("cn").unwrap();
        let acct = store.oauth_public();
        assert_eq!(acct.site.as_deref(), Some("cn"));
        assert_eq!(acct.site_url.as_deref(), Some("https://ariacompute.cn"));
        assert_eq!(
            acct.gateway_url.as_deref(),
            Some("https://gateway.ariacompute.cn")
        );
        // Invalid site fails closed.
        let err = store.oauth_set_site("mars").unwrap_err();
        assert!(err.to_string().contains("must be com|cn"), "{err}");

        // oauth_clear_api_key only drops the secret + prefix.
        store
            .oauth_set_api_key("sk-bf-CLEARME1234567", Some("k"))
            .unwrap();
        assert!(store.oauth_api_key().is_some());
        store.oauth_clear_api_key().unwrap();
        assert!(store.oauth_api_key().is_none());
        let acct = store.oauth_public();
        assert_eq!(acct.api_key_prefix.as_deref(), Some(""));
        assert!(acct.linked, "account stays linked");

        // Mark/unmark deletion requires a linked account to exist.
        store.oauth_mark_api_key_deleted().unwrap();
        assert!(store.oauth_public().api_key_deleted);
        store.oauth_unmark_api_key_deleted().unwrap();
        assert!(!store.oauth_public().api_key_deleted);

        // Unlinking revokes the OAuth row: several APIs then report "not linked".
        store.oauth_clear().unwrap();
        let cleared = store.oauth_public();
        assert!(!cleared.linked);
        assert_eq!(cleared.status, "not linked");
        assert!(store.oauth_clear_api_key().is_err());
        assert!(store.oauth_mark_api_key_deleted().is_err());
        assert!(store.oauth_unmark_api_key_deleted().is_err());
    }

    #[test]
    fn oauth_site_only_row_reports_key_only_status() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        // `oauth_set_api_key` on an unlinked store creates a key-only row.
        store
            .oauth_set_api_key("sk-bf-ORPHAN1234567", None)
            .unwrap();
        let acct = store.oauth_public();
        assert!(!acct.linked);
        assert!(acct.api_key_configured);
        assert_eq!(acct.status, "key only, not linked");
        assert_eq!(acct.api_key_name.as_deref(), Some("aria-router"));
        // A site can be attached to the same row afterwards.
        store.oauth_set_site("ariacompute.cn").unwrap();
        assert_eq!(store.oauth_public().site.as_deref(), Some("cn"));
        assert_eq!(
            store.oauth_site_url().as_deref(),
            Some("https://ariacompute.cn")
        );
    }

    #[test]
    fn begin_link_omits_empty_email_and_name_and_expires_state() {
        let dir = tempdir().unwrap();
        let store = KeyStore::empty(dir.path().join("keys.json"));
        let (url, state, site_url) = store
            .begin_link("com", None, Some(String::new()), Some(String::new()))
            .unwrap();
        assert!(!url.contains("&email="), "{url}");
        assert!(!url.contains("&name="), "{url}");
        assert_eq!(site_url, "https://ariacompute.com");
        assert_eq!(state.len(), 32);

        // Expired pending state is rejected (and consumed).
        store
            .pending
            .lock()
            .unwrap()
            .get_mut(&state)
            .unwrap()
            .expires_unix = now_unix() - 1;
        let err = store.take_pending(&state).unwrap_err();
        assert!(err.to_string().contains("oauth state expired"), "{err}");
        let err = store.take_pending("unknown-state").unwrap_err();
        assert!(err.to_string().contains("invalid oauth state"), "{err}");
    }

    #[test]
    fn apply_exchange_tolerates_unknown_site_and_rejects_bad_key() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        store
            .apply_exchange(ExchangeInput {
                site: "cn-mirror".into(),
                site_url: "https://custom.example".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::Number(7.into()),
                    email: None,
                    role: None,
                },
                link_token: None,
                expires_at: Some("2030-01-01T00:00:00Z".into()),
                api_key: None,
                owner_user_id: None,
            })
            .unwrap();
        let acct = store.oauth_public();
        // Unknown site with "cn" in it is treated as the CN deployment.
        assert_eq!(acct.site.as_deref(), Some("cn"));
        assert_eq!(acct.site_url.as_deref(), Some("https://custom.example"));
        assert!(acct.linked);

        // A second exchange reuses the existing OAuth row instead of adding one.
        store
            .apply_exchange(ExchangeInput {
                site: "intl".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::String("serve-2".into()),
                    email: Some("b@c.io".into()),
                    role: None,
                },
                link_token: Some("lt2".into()),
                expires_at: None,
                api_key: Some(("renamed".into(), "sk-bf-SECONDKEY12345".into())),
                owner_user_id: Some("usr_2".into()),
            })
            .unwrap();
        assert_eq!(
            store.oauth_api_key().as_deref(),
            Some("sk-bf-SECONDKEY12345")
        );
        assert_eq!(
            store.oauth_public().api_key_name.as_deref(),
            Some("renamed")
        );
        assert_eq!(store.oauth_owner_user_id().as_deref(), Some("usr_2"));

        let err = store
            .apply_exchange(ExchangeInput {
                site: "intl".into(),
                site_url: "https://ariacompute.com".into(),
                user: ServeUserInfo {
                    id: serde_json::Value::Null,
                    email: None,
                    role: None,
                },
                link_token: None,
                expires_at: None,
                api_key: Some(("bad".into(), "sk-aria_local".into())),
                owner_user_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, RouterError::InvalidParam(_)), "{err}");
    }

    #[test]
    fn normalize_site_and_gateway_endpoints() {
        assert_eq!(normalize_site("intl").unwrap().0, "intl");
        assert_eq!(normalize_site("COM").unwrap().1, "https://ariacompute.com");
        assert_eq!(normalize_site("1").unwrap().0, "intl");
        assert_eq!(normalize_site("ariacompute.com").unwrap().0, "intl");
        assert_eq!(normalize_site("cn").unwrap().0, "cn");
        assert_eq!(normalize_site("2").unwrap().0, "cn");
        assert_eq!(normalize_site("ariacompute.cn").unwrap().0, "cn");
        assert_eq!(
            normalize_site("https://ariacompute.cn/x").unwrap().1,
            "https://ariacompute.cn"
        );
        assert_eq!(
            normalize_site("https://ariacompute.com/x").unwrap().2,
            "https://gateway.ariacompute.com"
        );
        assert!(normalize_site("example.org").is_err());
        assert_eq!(gateway_for_site("cn"), "https://gateway.ariacompute.cn");
        assert_eq!(gateway_for_site("intl"), "https://gateway.ariacompute.com");
        assert_eq!(pct_encode_for_test("a b/c"), "a%20b%2Fc");
    }

    fn pct_encode_for_test(s: &str) -> String {
        KeyStore::pct_encode(s)
    }

    #[test]
    fn validate_oauth_key_rejects_foreign_and_short_keys() {
        let err = validate_oauth_key("sk-aria_abc").unwrap_err();
        assert!(err.to_string().contains("Local router key"), "{err}");
        let err = validate_oauth_key("plain-key").unwrap_err();
        assert!(err.to_string().contains("must start with sk-bf-"), "{err}");
        let err = validate_oauth_key("sk-bf-").unwrap_err();
        assert!(err.to_string().contains("too short"), "{err}");
    }

    #[test]
    fn extract_bearer_prefers_authorization_then_x_api_key() {
        use axum::http::header::AUTHORIZATION;
        let mut h = axum::http::HeaderMap::new();
        h.insert(AUTHORIZATION, "Bearer tok-1".parse().unwrap());
        assert_eq!(extract_bearer(&h).as_deref(), Some("tok-1"));
        h.insert(AUTHORIZATION, "Bearer   ".parse().unwrap());
        assert!(extract_bearer(&h).is_none());
        h.insert(AUTHORIZATION, "Basic abc".parse().unwrap());
        h.insert("x-api-key", "fallback".parse().unwrap());
        assert_eq!(extract_bearer(&h).as_deref(), Some("fallback"));
        h.insert("x-api-key", "   ".parse().unwrap());
        assert!(extract_bearer(&h).is_none());
        // Non-UTF-8 header values are ignored rather than panicking.
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            axum::http::HeaderValue::from_bytes(b"\x80").unwrap(),
        );
        assert!(extract_bearer(&h).is_none());
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            AUTHORIZATION,
            axum::http::HeaderValue::from_bytes(b"Bearer \x80").unwrap(),
        );
        assert!(extract_bearer(&h).is_none());
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            "x-api-key",
            axum::http::HeaderValue::from_bytes(b"\x80").unwrap(),
        );
        assert!(extract_bearer(&h).is_none());
    }

    #[test]
    fn load_skips_migration_when_oauth_row_exists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let mut store = linked_store(&dir);
        store
            .oauth_set_api_key("sk-bf-PERSIST1234567", None)
            .unwrap();
        // Legacy file present, but an active OAuth row already wins.
        std::fs::write(
            dir.path().join("router-serve.json"),
            r#"{"site":"com","api_key":"sk-bf-LEGACY12345678"}"#,
        )
        .unwrap();
        let reloaded = KeyStore::load(&path).unwrap();
        assert_eq!(
            reloaded.oauth_api_key().as_deref(),
            Some("sk-bf-PERSIST1234567")
        );
        assert!(dir.path().join("router-serve.json").exists());
    }

    #[test]
    fn legacy_migration_keeps_user_only_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("router-keys.json");
        std::fs::write(
            dir.path().join("router-serve.json"),
            r#"{"site":"cn","user":{"id":"serve-9","email":"a@b.io"}}"#,
        )
        .unwrap();
        let store = KeyStore::load(&path).unwrap();
        let acct = store.oauth_public();
        assert!(acct.linked);
        // No api_key in the legacy file → nothing configured, name/prefix defaults.
        assert!(!acct.api_key_configured);
        assert_eq!(acct.api_key_name.as_deref(), Some("aria-router"));
        assert_eq!(acct.api_key_prefix.as_deref(), Some(""));
        assert_eq!(acct.status, "linked, no api key");
    }

    #[test]
    fn legacy_migration_ignores_empty_files_and_bad_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("router-keys.json");
        std::fs::write(dir.path().join("router-serve.json"), "{}").unwrap();
        let store = KeyStore::load(&path).unwrap();
        assert!(!store.oauth_public().linked);
        // The legacy file is removed even when empty.
        assert!(!dir.path().join("router-serve.json").exists());

        std::fs::write(dir.path().join("router-serve.json"), "{not json").unwrap();
        let err = KeyStore::load(&path).unwrap_err();
        assert!(matches!(err, RouterError::Config(_)), "{err}");
    }

    #[test]
    fn persist_reports_unwritable_paths() {
        // A regular file as a path component cannot be a directory on any
        // platform (`/proc/...` was Linux-only, and is writable on Windows).
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let mut store = KeyStore::empty(blocker.join("keys.json"));
        let err = store.create("ci").unwrap_err();
        assert!(matches!(err, RouterError::Io(_)), "{err}");
    }
    #[test]
    fn persist_creates_missing_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/router-keys.json");
        let mut store = KeyStore::empty(path.clone());
        store.create("k").unwrap();
        assert!(path.is_file(), "persist must mkdir parents");
    }

    #[test]
    fn apply_exchange_falls_back_for_unknown_site_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let info = |site: &str, url: &str| ExchangeInput {
            site: site.into(),
            site_url: url.into(),
            user: ServeUserInfo {
                id: serde_json::Value::String("s1".into()),
                email: None,
                role: None,
            },
            link_token: None,
            expires_at: None,
            api_key: None,
            owner_user_id: None,
        };
        // `contains("cn")` selects the CN site, everything else the intl site.
        store
            .apply_exchange(info("https://serve.internal.cn", "https://a"))
            .unwrap();
        assert_eq!(store.oauth_public().site.as_deref(), Some("cn"));
        store.oauth_clear().unwrap();
        store
            .apply_exchange(info("https://serve.internal", "https://b"))
            .unwrap();
        assert_eq!(store.oauth_public().site.as_deref(), Some("intl"));
    }

    #[test]
    fn extract_bearer_rejects_empty_tokens() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("authorization", "Bearer    ".parse().unwrap());
        assert!(extract_bearer(&h).is_none());
        h.insert("authorization", "Bearer tok".parse().unwrap());
        assert_eq!(extract_bearer(&h).as_deref(), Some("tok"));
    }

    #[test]
    fn calendar_rollover_resets_the_whole_shared_group() {
        let dir = tempdir().unwrap();
        let mut store = KeyStore::empty(dir.path().join("keys.json"));
        let a = store.create("a").unwrap();
        let b = store.create("b").unwrap();
        for k in [&a, &b] {
            store
                .set_spend_policy(
                    &k.id,
                    Some(5.0),
                    Some("team".into()),
                    Some("calendar_month".into()),
                )
                .unwrap();
        }
        store.record_spend(&a.id, 2.0, "2026-10").unwrap();
        store.record_spend(&b.id, 1.0, "2026-10").unwrap();
        assert_eq!(
            store.spend_remaining(&a.id, "2026-10", 0.0).unwrap(),
            Some(2.0)
        );
        // A new period resets every member of the shared group, not just `a`.
        assert_eq!(
            store.spend_remaining(&a.id, "2026-11", 0.0).unwrap(),
            Some(5.0)
        );
        assert_eq!(
            store.spend_remaining(&b.id, "2026-11", 0.0).unwrap(),
            Some(5.0)
        );
    }
}
