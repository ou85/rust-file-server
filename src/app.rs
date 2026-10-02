use crate::{
    auth::{SessionStore, hash_password},
    blob_store::{ChunkIterator, RangeChunkIterator, Storage},
    config::Config,
    crypto::Crypto,
    domain::{AuthRecord, FileMetadata},
    metadata::MetadataStore,
};

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use std::error::Error;
use std::fs;
use std::sync::Arc;

pub struct App {
    pub config: Config,
    pub storage: Arc<Storage>,
    pub metadata: MetadataStore,
    pub crypto: Crypto,
    pub sessions: SessionStore,
}

impl App {
    pub fn auth_record(&self) -> Result<crate::domain::AuthRecord, Box<dyn std::error::Error>> {
        self.metadata
            .auth_record()?
            .ok_or_else(|| "User account is not initialized".into())
    }

    pub fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        ensure_dir_exists(&config.data_dir)?;
        ensure_dir_exists(&config.blobs_dir)?;
        ensure_dir_exists(&config.tmp_dir)?;

        let storage = Arc::new(Storage::new(
            config.blobs_dir.clone(),
            config.tmp_dir.clone(),
        )?);
        let metadata = MetadataStore::new(&config)?;
        if metadata.auth_record()?.is_none() {
            let password_hash = match config.bootstrap_password.as_deref() {
                Some(password) if password.trim().len() >= crate::auth::MIN_PASSWORD_LEN => {
                    hash_password(password).map_err(|error| error.to_string())?
                }
                Some(_) => {
                    return Err(format!(
                        "RFS_BOOTSTRAP_PASSWORD must contain at least {} characters",
                        crate::auth::MIN_PASSWORD_LEN
                    )
                    .into());
                }
                None => config
                    .user_password_hash
                    .clone()
                    .ok_or("RFS_BOOTSTRAP_PASSWORD is required for first startup")?,
            };
            metadata.save_auth_record(&AuthRecord {
                username: "user".into(),
                password_hash,
                auth_version: 1,
            })?;
            tracing::info!("Initial user account created");
        }
        let crypto = Crypto::new(&config.encryption_key)?;

        match storage.cleanup_all_tmp_files() {
            Ok(count) if count > 0 => tracing::info!(count, "Orphaned temporary files removed"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Temporary file cleanup failed"),
        }
        let known_ids = metadata
            .list_files()?
            .into_iter()
            .map(|file| file.id)
            .collect();
        match storage.cleanup_orphaned_files(&known_ids) {
            Ok(count) if count > 0 => tracing::info!(count, "Orphaned blobs removed"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Orphaned blob cleanup failed"),
        }

        Ok(Self {
            config,
            storage,
            metadata,
            crypto,
            sessions: SessionStore::new(),
        })
    }

    pub fn print_banner(addr: &str, database_path: &Path) -> std::io::Result<()> {
        use std::io::{IsTerminal, Write};

        let stdout = std::io::stdout();
        let (blue, reset) = if stdout.is_terminal() {
            ("\x1b[36m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut output = stdout.lock();
        writeln!(output)?;
        writeln!(output, "──────────────────────────────")?;
        writeln!(output, "rust-file-server v{}", env!("CARGO_PKG_VERSION"))?;
        writeln!(output, "──────────────────────────────")?;
        writeln!(output, "✓ Database:  {}", database_path.display())?;
        writeln!(output)?;
        writeln!(output, "→ Server online")?;
        writeln!(output, "→ Listening on {blue}http://{addr}{reset}")?;
        writeln!(output)
    }

    pub fn import_file(&self, path: &str) -> Result<FileMetadata, Box<dyn std::error::Error>> {
        let file = Storage::import_file(path)?;

        let metadata = FileMetadata {
            id: file.id.clone(),
            filename: file.filename.clone(),
            size: file.content.len() as u64,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };

        self.storage.save_file(&file, &self.crypto)?;
        if let Err(error) = self.metadata.save_file(&metadata) {
            let _ = self.storage.delete_file(&file.id);
            return Err(error);
        }

        Ok(metadata)
    }

    pub fn get_file(&self, id: &str) -> Result<Option<FileMetadata>, Box<dyn std::error::Error>> {
        self.metadata.get_file(id)
    }

    pub fn list_files(&self) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let mut files = self.metadata.list_files()?;

        files.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(files)
    }

    pub fn delete_file(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Attempt to delete the file from the disk, but do not fail if the file is missing.
        match self.storage.delete_file(id) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Box::new(e)),
        }

        self.metadata.delete_file(id)?;

        Ok(())
    }

    pub fn demo(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let metadata = self.import_file(path)?;

        println!("\n=== Imported: {} ({})", metadata.filename, metadata.id);

        for file in self.list_files()? {
            println!("{} | {} | {} bytes", file.id, file.filename, file.size,);
        }

        Ok(())
    }

    /// For download/stream - returns an iterator over chunks
    pub fn export_chunked(&self, id: &str) -> Result<ChunkIterator, Box<dyn std::error::Error>> {
        Ok(self.storage.stream_chunks(id, &self.crypto)?)
    }

    pub fn export_range(
        &self,
        id: &str,
        byte_start: u64,
        byte_end: u64,
    ) -> Result<RangeChunkIterator, Box<dyn std::error::Error>> {
        Ok(self
            .storage
            .stream_chunks_range(id, &self.crypto, byte_start, byte_end)?)
    }
}

fn ensure_dir_exists<P: AsRef<Path>>(path: P) -> Result<(), Box<dyn Error>> {
    let p = path.as_ref();
    if !p.exists() {
        fs::create_dir_all(p)?;
        tracing::info!(path = %p.display(), "Directory created");
    }
    set_owner_only_permissions(p)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
}
