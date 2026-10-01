use crate::config::Config;
use bcrypt::verify;
use rand::Rng;
use std::{collections::HashMap, sync::Mutex};

pub fn authenticate(username: &str, password: &str, config: &Config) -> Option<UserRole> {
    match username {
        u if u == config.user_name => {
            if verify(password, &config.user_password_hash).ok()? {
                Some(UserRole::User)
            } else {
                None
            }
        }
        a if a == config.admin_name => {
            if verify(password, &config.admin_password_hash).ok()? {
                Some(UserRole::Admin)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UserRole {
    User,
    Admin,
}

pub struct SessionStore {
    sessions: Mutex<HashMap<String, UserRole>>,
}

#[derive(Debug)]
pub struct SessionError;

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session storage lock is poisoned")
    }
}

impl std::error::Error for SessionError {}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn create(&self, role: UserRole) -> Result<String, SessionError> {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        self.sessions
            .lock()
            .map_err(|_| SessionError)?
            .insert(token.clone(), role);
        Ok(token)
    }

    pub fn role(&self, token: &str) -> Option<UserRole> {
        self.sessions.lock().ok()?.get(token).copied()
    }

    pub fn remove(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_session_storage_returns_error_instead_of_panicking() {
        let sessions = SessionStore::new();
        let _ = std::panic::catch_unwind(|| {
            let _guard = sessions.sessions.lock().unwrap();
            panic!("Simulated failure while holding the session lock");
        });
        assert!(sessions.create(UserRole::User).is_err());
        assert_eq!(sessions.role("forged"), None);
    }

    #[test]
    fn session_token_is_server_side_and_revocable() {
        let sessions = SessionStore::new();
        let token = sessions.create(UserRole::User).unwrap();
        assert_eq!(sessions.role(&token), Some(UserRole::User));
        assert_eq!(sessions.role("forged"), None);
        sessions.remove(&token);
        assert_eq!(sessions.role(&token), None);
    }
}
