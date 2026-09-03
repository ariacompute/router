//! Local Dashboard users: username/password (argon2id) + opaque sessions.

use aria_router_core::RouterError;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SESSION_TTL_SECS: u64 = 12 * 3600;
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::User => "user",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: String,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UserFile {
    #[serde(default)]
    users: Vec<UserRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: String,
    pub username: String,
    pub role: UserRole,
    pub created_at: String,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
struct Session {
    user_id: String,
    expires_unix: u64,
}

#[derive(Debug)]
pub struct UserStore {
    path: PathBuf,
    users: Vec<UserRecord>,
    sessions: HashMap<String, Session>,
}

impl UserStore {
    pub fn load(path: &Path) -> Result<Self, RouterError> {
        let users = if path.exists() {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                RouterError::Io(format!("read {}: {e}", path.display()))
            })?;
            let file: UserFile = serde_json::from_str(&raw).map_err(|e| {
                RouterError::Config(format!("users file {}: {e}", path.display()))
            })?;
            file.users
        } else {
            Vec::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            users,
            sessions: HashMap::new(),
        })
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            users: Vec::new(),
            sessions: HashMap::new(),
        }
    }

    fn persist(&self) -> Result<(), RouterError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RouterError::Io(format!("mkdir {}: {e}", parent.display()))
            })?;
        }
        let file = UserFile {
            users: self.users.clone(),
        };
        let raw = serde_json::to_string_pretty(&file).map_err(|e| RouterError::Io(e.to_string()))?;
        std::fs::write(&self.path, raw).map_err(|e| {
            RouterError::Io(format!("write {}: {e}", self.path.display()))
        })?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    pub fn counts(&self) -> (usize, usize) {
        let admin = self
            .users
            .iter()
            .filter(|u| matches!(u.role, UserRole::Admin) && !u.disabled)
            .count();
        let user = self
            .users
            .iter()
            .filter(|u| matches!(u.role, UserRole::User) && !u.disabled)
            .count();
        (admin, user)
    }

    pub fn list_public(&self) -> Vec<UserPublic> {
        self.users.iter().map(UserPublic::from).collect()
    }

    pub fn get(&self, id: &str) -> Option<&UserRecord> {
        self.users.iter().find(|u| u.id == id)
    }

    pub fn create_admin(path: &Path, username: &str, password: &str) -> Result<UserPublic, RouterError> {
        let mut store = if path.exists() {
            Self::load(path)?
        } else {
            Self::empty(path.to_path_buf())
        };
        if !store.users.is_empty() {
            return Err(RouterError::InvalidParam(
                "users already exist; skip admin bootstrap or clear users file".into(),
            ));
        }
        let u = store.insert_user(username, password, UserRole::Admin)?;
        Ok(u)
    }

    fn insert_user(
        &mut self,
        username: &str,
        password: &str,
        role: UserRole,
    ) -> Result<UserPublic, RouterError> {
        let username = username.trim();
        if username.is_empty() {
            return Err(RouterError::InvalidParam("username required".into()));
        }
        if password.len() < MIN_PASSWORD_LEN {
            return Err(RouterError::InvalidParam(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        if self
            .users
            .iter()
            .any(|u| u.username.eq_ignore_ascii_case(username))
        {
            return Err(RouterError::InvalidParam("username already taken".into()));
        }
        let hash = hash_password(password)?;
        let mut rnd = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut rnd);
        let id = format!("usr_{}", hex::encode(rnd));
        let created_at = now_rfc3339();
        let rec = UserRecord {
            id: id.clone(),
            username: username.to_string(),
            password_hash: hash,
            role: role.clone(),
            created_at: created_at.clone(),
            disabled: false,
        };
        self.users.push(rec);
        self.persist()?;
        Ok(UserPublic {
            id,
            username: username.to_string(),
            role,
            created_at,
            disabled: false,
        })
    }

    pub fn register(
        &mut self,
        username: &str,
        password: &str,
        allow_register: bool,
    ) -> Result<(UserPublic, String), RouterError> {
        if self.users.is_empty() {
            return Err(RouterError::FailClosed(
                "no local users yet; run aria-router setup first".into(),
            ));
        }
        if !allow_register {
            return Err(RouterError::Unauthorized("registration disabled".into()));
        }
        let pubu = self.insert_user(username, password, UserRole::User)?;
        let token = self.issue_session(&pubu.id)?;
        Ok((pubu, token))
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<(UserPublic, String), RouterError> {
        let username = username.trim();
        let (pubu, user_id) = {
            let Some(u) = self
                .users
                .iter()
                .find(|u| u.username.eq_ignore_ascii_case(username))
            else {
                return Err(RouterError::Unauthorized("invalid credentials".into()));
            };
            if u.disabled {
                return Err(RouterError::Unauthorized("user disabled".into()));
            }
            verify_password(password, &u.password_hash)?;
            (UserPublic::from(u), u.id.clone())
        };
        let token = self.issue_session(&user_id)?;
        Ok((pubu, token))
    }

    pub fn issue_session(&mut self, user_id: &str) -> Result<String, RouterError> {
        let mut rnd = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut rnd);
        let token = hex::encode(rnd);
        let expires_unix = now_unix() + SESSION_TTL_SECS;
        self.sessions.insert(
            token.clone(),
            Session {
                user_id: user_id.to_string(),
                expires_unix,
            },
        );
        Ok(token)
    }

    pub fn logout(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    pub fn resolve_session(&mut self, token: &str) -> Result<UserPublic, RouterError> {
        let now = now_unix();
        let Some(sess) = self.sessions.get(token) else {
            return Err(RouterError::Unauthorized("invalid session".into()));
        };
        if sess.expires_unix < now {
            self.sessions.remove(token);
            return Err(RouterError::Unauthorized("session expired".into()));
        }
        let user_id = sess.user_id.clone();
        let Some(u) = self.users.iter().find(|u| u.id == user_id) else {
            return Err(RouterError::Unauthorized("invalid session".into()));
        };
        if u.disabled {
            return Err(RouterError::Unauthorized("user disabled".into()));
        }
        Ok(UserPublic::from(u))
    }

    pub fn set_disabled(&mut self, id: &str, disabled: bool) -> Result<(), RouterError> {
        let Some(u) = self.users.iter_mut().find(|u| u.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown user {id}")));
        };
        u.disabled = disabled;
        self.persist()?;
        Ok(())
    }

    pub fn set_password(&mut self, id: &str, password: &str) -> Result<(), RouterError> {
        if password.len() < MIN_PASSWORD_LEN {
            return Err(RouterError::InvalidParam(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )));
        }
        let hash = hash_password(password)?;
        let Some(u) = self.users.iter_mut().find(|u| u.id == id) else {
            return Err(RouterError::InvalidParam(format!("unknown user {id}")));
        };
        u.password_hash = hash;
        self.persist()?;
        Ok(())
    }
}

impl From<&UserRecord> for UserPublic {
    fn from(u: &UserRecord) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            role: u.role.clone(),
            created_at: u.created_at.clone(),
            disabled: u.disabled,
        }
    }
}

fn hash_password(password: &str) -> Result<String, RouterError> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| RouterError::Io(format!("password hash: {e}")))?;
    Ok(hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> Result<(), RouterError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|_| RouterError::Unauthorized("invalid credentials".into()))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| RouterError::Unauthorized("invalid credentials".into()))
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

pub fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(axum::http::header::COOKIE) {
        if let Ok(s) = v.to_str() {
            for part in s.split(';') {
                let part = part.trim();
                if let Some(rest) = part.strip_prefix("aria_router_session=") {
                    let t = rest.trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    if let Some(v) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            let s = s.trim();
            if let Some(rest) = s.strip_prefix("Bearer ") {
                let t = rest.trim();
                // Session tokens are hex; sk-aria_ / bfvk- are API keys — ignore those here.
                if !t.is_empty() && !t.starts_with("sk-aria_") && !t.starts_with("bfvk-") {
                    return Some(t.to_string());
                }
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
    fn admin_register_login() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("users.json");
        UserStore::create_admin(&path, "admin", "password1").unwrap();
        let mut store = UserStore::load(&path).unwrap();
        let (u, tok) = store
            .register("alice", "password1", true)
            .unwrap();
        assert_eq!(u.role, UserRole::User);
        let me = store.resolve_session(&tok).unwrap();
        assert_eq!(me.username, "alice");
        assert!(store.register("bob", "password1", false).is_err());
    }

    #[test]
    fn register_without_admin_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("users.json");
        let mut store = UserStore::empty(path);
        let err = store.register("alice", "password1", true).unwrap_err();
        assert!(matches!(err, RouterError::FailClosed(_)));
    }
}
