//! API key store: Dashboard-issued secrets, sha256 on disk.

use ariarouter_core::RouterError;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SECRET_PREFIX: &str = "sk-aria_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRecord {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub secret_sha256: String,
    pub created_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeyFile {
    #[serde(default)]
    keys: Vec<KeyRecord>,
}

#[derive(Debug, Clone)]
pub struct KeyStore {
    path: PathBuf,
    keys: Vec<KeyRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyPublic {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyCreated {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub secret: String,
    pub created_at: String,
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
        Ok(Self {
            path: path.to_path_buf(),
            keys,
        })
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            keys: Vec::new(),
        }
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
        Ok(())
    }

    pub fn list_public(&self) -> Vec<KeyPublic> {
        self.keys
            .iter()
            .map(|k| KeyPublic {
                id: k.id.clone(),
                name: k.name.clone(),
                prefix: k.prefix.clone(),
                created_at: k.created_at.clone(),
                last_used_at: k.last_used_at.clone(),
                revoked: k.revoked,
            })
            .collect()
    }

    pub fn counts(&self) -> (usize, usize) {
        let active = self.keys.iter().filter(|k| !k.revoked).count();
        let revoked = self.keys.iter().filter(|k| k.revoked).count();
        (active, revoked)
    }

    pub fn create(&mut self, name: &str) -> Result<KeyCreated, RouterError> {
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
            id: id.clone(),
            name: name.to_string(),
            prefix: prefix.clone(),
            secret_sha256: hash,
            created_at: created_at.clone(),
            last_used_at: None,
            revoked: false,
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

    pub fn revoke(&mut self, id: &str) -> Result<(), RouterError> {
        let Some(k) = self.keys.iter_mut().find(|k| k.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown key id {id}")));
        };
        k.revoked = true;
        self.persist()?;
        Ok(())
    }

    /// Verify secret; on success update last_used_at and return (id, name).
    pub fn authenticate(&mut self, secret: &str) -> Result<(String, String), RouterError> {
        let hash = sha256_hex(secret.trim());
        let Some(k) = self.keys.iter_mut().find(|k| k.secret_sha256 == hash) else {
            return Err(RouterError::Unauthorized("invalid api key".into()));
        };
        if k.revoked {
            return Err(RouterError::Unauthorized("api key revoked".into()));
        }
        k.last_used_at = Some(now_rfc3339());
        let id = k.id.clone();
        let name = k.name.clone();
        let _ = self.persist();
        Ok((id, name))
    }
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
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
        let (id, name) = store.authenticate(&created.secret).unwrap();
        assert_eq!(id, created.id);
        assert_eq!(name, "ci");
        store.revoke(&created.id).unwrap();
        assert!(store.authenticate(&created.secret).is_err());
    }
}
