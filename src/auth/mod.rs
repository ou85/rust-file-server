use crate::domain::{UserAccount, UserRole};
use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{PasswordHasher, SaltString},
};
use rand::Rng;
use std::{collections::HashMap, sync::Mutex};

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn validate_username(username: &str) -> Result<(), &'static str> {
    if !(3..=64).contains(&username.len())
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(
            "Username must be 3-64 characters and contain only letters, numbers, '.', '_' or '-'",
        );
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, Box<dyn std::error::Error>> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(format!("Password must contain at least {MIN_PASSWORD_LEN} characters").into());
    }
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let salt = SaltString::encode_b64(&bytes).map_err(|error| error.to_string())?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| error.to_string())?
        .to_string())
}

pub fn verify_password(password: &str, account: &UserAccount) -> bool {
    PasswordHash::new(&account.password_hash)
        .ok()
        .is_some_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
}

#[derive(Clone, Debug)]
pub struct Session {
    pub user_id: String,
    pub role: UserRole,
    pub auth_version: u64,
}

pub struct SessionStore {
    sessions: Mutex<HashMap<String, Session>>,
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

    pub fn create(&self, account: &UserAccount) -> Result<String, SessionError> {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        self.sessions.lock().map_err(|_| SessionError)?.insert(
            token.clone(),
            Session {
                user_id: account.id.clone(),
                role: account.role,
                auth_version: account.auth_version,
            },
        );
        Ok(token)
    }

    pub fn get(&self, token: &str) -> Option<Session> {
        self.sessions.lock().ok()?.get(token).cloned()
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

    fn account() -> UserAccount {
        UserAccount {
            id: "user-id".into(),
            username: "user".into(),
            password_hash: hash_password("correct horse battery staple").unwrap(),
            auth_version: 7,
            role: UserRole::User,
            password_change_required: false,
        }
    }

    #[test]
    fn argon2id_passwords_and_versioned_sessions_work() {
        let account = account();
        assert!(verify_password("correct horse battery staple", &account));
        assert!(!verify_password("wrong password", &account));
        let sessions = SessionStore::new();
        let token = sessions.create(&account).unwrap();
        assert_eq!(sessions.get(&token).unwrap().auth_version, 7);
        assert_eq!(sessions.get(&token).unwrap().user_id, "user-id");
    }

    #[test]
    fn usernames_are_restricted_to_safe_characters() {
        assert!(validate_username("user-2").is_ok());
        assert!(validate_username("no spaces").is_err());
        assert!(validate_username("ab").is_err());
    }
}
