use serde::{Deserialize, Serialize};
pub struct StoredFile {
    pub id: String,
    pub filename: String,
    pub content: Vec<u8>, //  Vec<T> - vector of T, dynamic array of T (Heap-allocated list)
}

#[derive(Serialize, Deserialize)]
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
