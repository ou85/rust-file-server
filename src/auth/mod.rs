use crate::{config::Config, domain::AuthRecord};
use argon2::{
    Argon2, PasswordHash, PasswordVerifier,
    password_hash::{PasswordHasher, SaltString},
};
use bcrypt::verify as bcrypt_verify;
use rand::Rng;
use std::{collections::HashMap, sync::Mutex};

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn authenticate(username: &str, password: &str, config: &Config) -> Option<UserRole> {
    match username {
        u if u == config.user_name => {
            if config
                .user_password_hash
                .as_deref()
                .is_some_and(|hash| bcrypt_verify(password, hash).ok().unwrap_or(false))
            {
                Some(UserRole::User)
            } else {
                None
            }
        }
        a if a == config.admin_name => {
            if config
                .admin_password_hash
                .as_deref()
                .is_some_and(|hash| bcrypt_verify(password, hash).ok().unwrap_or(false))
            {
                Some(UserRole::Admin)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let salt = SaltString::encode_b64(&bytes)?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, record: &AuthRecord) -> bool {
    if record.password_hash.starts_with("$argon2") {
        return PasswordHash::new(&record.password_hash)
            .ok()
            .is_some_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            });
    }
    bcrypt_verify(password, &record.password_hash)
        .ok()
        .unwrap_or(false)
}

pub fn is_argon2_hash(record: &AuthRecord) -> bool {
    record.password_hash.starts_with("$argon2")
}

pub fn change_password(
    metadata: &crate::metadata::MetadataStore,
    current: Option<&str>,
    next: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if next.trim().len() < MIN_PASSWORD_LEN {
        return Err(format!("Password must contain at least {MIN_PASSWORD_LEN} characters").into());
    }
    let mut record = metadata
        .auth_record()?
        .ok_or("User account is not initialized")?;
    if let Some(current) = current {
        if !verify_password(current, &record) {
            return Err("Current password is incorrect".into());
        }
    }
    record.password_hash = hash_password(next).map_err(|error| error.to_string())?;
    record.auth_version = record
        .auth_version
        .checked_add(1)
        .ok_or("Authentication version exhausted")?;
    metadata.save_auth_record(&record)?;
    Ok(())
}

pub fn rename_user(
    metadata: &crate::metadata::MetadataStore,
    current: &str,
    new_username: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let new_username = new_username.trim();
    if !(3..=64).contains(&new_username.len())
        || !new_username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(
            "Username must be 3-64 characters and contain only letters, numbers, '.', '_' or '-'"
                .into(),
        );
    }
    let mut record = metadata
        .auth_record()?
        .ok_or("User account is not initialized")?;
    if !verify_password(current, &record) {
        return Err("Current password is incorrect".into());
    }
    record.username = new_username.to_owned();
    record.auth_version = record
        .auth_version
        .checked_add(1)
        .ok_or("Authentication version exhausted")?;
    metadata.save_auth_record(&record)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UserRole {
    User,
    Admin,
}

pub struct SessionStore {
    sessions: Mutex<HashMap<String, (UserRole, u64)>>,
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

    pub fn create(&self, role: UserRole, auth_version: u64) -> Result<String, SessionError> {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        self.sessions
            .lock()
            .map_err(|_| SessionError)?
            .insert(token.clone(), (role, auth_version));
        Ok(token)
    }

    pub fn role(&self, token: &str, auth_version: u64) -> Option<UserRole> {
        self.sessions
            .lock()
            .ok()?
            .get(token)
            .and_then(|(role, version)| (*version == auth_version).then_some(*role))
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
    fn argon2id_passwords_verify_and_auth_versions_revoke_sessions() {
        let hash = hash_password("correct horse battery staple").unwrap();
        let record = AuthRecord {
            username: "user".into(),
            password_hash: hash,
            auth_version: 7,
        };
        assert!(is_argon2_hash(&record));
        assert!(verify_password("correct horse battery staple", &record));
        assert!(!verify_password("wrong password", &record));
        let sessions = SessionStore::new();
        let token = sessions
            .create(UserRole::User, record.auth_version)
            .unwrap();
        assert_eq!(sessions.role(&token, 7), Some(UserRole::User));
        assert_eq!(sessions.role(&token, 8), None);
    }

    #[test]
    fn poisoned_session_storage_returns_error_instead_of_panicking() {
        let sessions = SessionStore::new();
        let _ = std::panic::catch_unwind(|| {
            let _guard = sessions.sessions.lock().unwrap();
            panic!("Simulated failure while holding the session lock");
        });
        assert!(sessions.create(UserRole::User, 1).is_err());
        assert_eq!(sessions.role("forged", 1), None);
    }

    #[test]
    fn session_token_is_server_side_and_revocable() {
        let sessions = SessionStore::new();
        let token = sessions.create(UserRole::User, 1).unwrap();
        assert_eq!(sessions.role(&token, 1), Some(UserRole::User));
        assert_eq!(sessions.role(&token, 2), None);
        assert_eq!(sessions.role("forged", 1), None);
        sessions.remove(&token);
        assert_eq!(sessions.role(&token, 1), None);
    }
}
