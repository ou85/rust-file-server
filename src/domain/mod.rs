use serde::{Deserialize, Serialize};

pub mod id;

pub struct StoredFile {
    pub id: String,
    pub filename: String,
    pub content: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub created_at: u64,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct BulkDeleteRequest {
    pub ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    User,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub auth_version: u64,
    pub role: UserRole,
    pub password_change_required: bool,
}
