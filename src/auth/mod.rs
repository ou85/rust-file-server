use crate::config::Config;
use bcrypt::verify;
use rand::Rng;
use std::{collections::HashMap, sync::Mutex};

pub fn authenticate(username: &str, password: &str, config: &Config) -> Option<UserRole> {
    // println!("Input username: {}", username);
    // println!("Input password: {}", password);
    // println!("Stored user hash: {}", &config.user_password_hash);
    // println!("Stored admin hash: {}", &config.admin_password_hash);

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

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn create(&self, role: UserRole) -> String {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        self.sessions
            .lock()
            .expect("session mutex poisoned")
            .insert(token.clone(), role);
        token
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
    fn session_token_is_server_side_and_revocable() {
        let sessions = SessionStore::new();
        let token = sessions.create(UserRole::User);
        assert_eq!(sessions.role(&token), Some(UserRole::User));
        assert_eq!(sessions.role("forged"), None);
        sessions.remove(&token);
        assert_eq!(sessions.role(&token), None);
    }
}
