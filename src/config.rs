pub struct Config {
    pub storage_path: String,
    pub database_path: String,
    pub encryption_key: String,

    pub user_name: String,
    pub admin_name: String,
    pub user_password_hash: String,
    pub admin_password_hash: String,

    pub max_preview_size_bytes: u64,
}

impl Config {
    pub fn new() -> Self {
        let max_preview_size_bytes = max_preview_size();

        Self {
            storage_path: "data/files".to_string(),
            database_path: "data/db".to_string(),

            encryption_key: std::env::var("RFS_ENCRYPTION_KEY")
                .expect("RFS_ENCRYPTION_KEY is not set"),
            user_name: "user".to_string(),
            user_password_hash: std::env::var("RFS_USER_PASSWORD_HASH")
                .expect("RFS_USER_PASSWORD_HASH is not set"),
            admin_name: "admin".to_string(),
            admin_password_hash: std::env::var("RFS_ADMIN_PASSWORD_HASH")
                .expect("RFS_ADMIN_PASSWORD_HASH is not set"),
            max_preview_size_bytes,
        }
    }
}

pub fn new_port() -> String {
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    format!("0.0.0.0:{}", port)
}

fn max_preview_size() -> u64 {
    let mb = match std::env::var("RFS_MAX_PREVIEW_SIZE_MB") {
        Ok(value) => match value.parse::<u64>() {
            Ok(size) => size,
            Err(_) => {
                eprintln!(
                    "Invalid RFS_MAX_PREVIEW_SIZE_MB value: {}, using 200",
                    value
                );
                200
            }
        },
        Err(_) => {
            eprintln!("RFS_MAX_PREVIEW_SIZE_MB not set, using 200");
            200
        }
    };

    mb * 1024 * 1024
}
