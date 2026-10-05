use crate::{
    auth::{Session, SessionStore, hash_password, validate_username},
    blob_store::{ChunkIterator, RangeChunkIterator, Storage},
    config::Config,
    crypto::Crypto,
    domain::{FileMetadata, FolderMetadata, UserAccount, UserRole},
    metadata::MetadataStore,
};

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use std::error::Error;
use std::fs;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

struct UploadState {
    owner_id: String,
    cancelled: bool,
    completed: bool,
    completed_at: Option<Instant>,
}

pub struct App {
    pub config: Config,
    pub storage: Arc<Storage>,
    pub metadata: MetadataStore,
    pub crypto: Crypto,
    pub sessions: SessionStore,
    uploads: Mutex<HashMap<String, UploadState>>,
}

impl App {
    fn prepare(
        config: &Config,
    ) -> Result<(Arc<Storage>, MetadataStore, Crypto), Box<dyn std::error::Error>> {
        ensure_dir_exists(&config.data_dir)?;
        ensure_dir_exists(&config.blobs_dir)?;
        ensure_dir_exists(&config.tmp_dir)?;
        Ok((
            Arc::new(Storage::new(
                config.blobs_dir.clone(),
                config.tmp_dir.clone(),
            )?),
            MetadataStore::new(config)?,
            Crypto::new(&config.encryption_key)?,
        ))
    }

    pub fn initialize(
        config: &Config,
        admin_username: &str,
        admin_password: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_username(admin_username)?;
        let (_, metadata, crypto) = Self::prepare(config)?;
        if metadata.user_count()? != 0 {
            return Err("Server is already initialized".into());
        }
        let account = UserAccount {
            id: uuid::Uuid::new_v4().to_string(),
            username: admin_username.to_owned(),
            password_hash: hash_password(admin_password)?,
            auth_version: 1,
            role: UserRole::Admin,
            password_change_required: false,
            encrypted_data_key: crypto.create_wrapped_user_key()?,
        };
        metadata.create_user(&account)?;
        let legacy_files = metadata.legacy_file_count()?;
        if legacy_files > 0 {
            tracing::warn!(
                legacy_files,
                "Legacy files were found and remain unassigned and inaccessible"
            );
        }
        tracing::info!(username = %admin_username, "Administrator account initialized");
        Ok(())
    }

    pub fn current_user(
        &self,
        token: &str,
    ) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let Some(Session {
            user_id,
            role,
            auth_version,
        }) = self.sessions.get(token)
        else {
            return Ok(None);
        };
        let Some(account) = self.metadata.get_user(&user_id)? else {
            return Ok(None);
        };
        Ok((account.role == role && account.auth_version == auth_version).then_some(account))
    }

    pub fn user_crypto(&self, user: &UserAccount) -> Result<Crypto, Box<dyn std::error::Error>> {
        Ok(self.crypto.for_wrapped_user_key(&user.encrypted_data_key)?)
    }

    pub fn user_root_folder(
        &self,
        user: &UserAccount,
    ) -> Result<FolderMetadata, Box<dyn std::error::Error>> {
        Ok(self.metadata.ensure_root_folder(&user.id)?)
    }

    pub fn user_folder(
        &self,
        user: &UserAccount,
        id: &str,
    ) -> Result<Option<FolderMetadata>, Box<dyn std::error::Error>> {
        Ok(self
            .metadata
            .get_folder(id)?
            .filter(|folder| folder.owner_id == user.id))
    }

    pub fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let (storage, metadata, crypto) = Self::prepare(&config)?;
        if metadata.user_count()? == 0 {
            return Err("Server is not initialized. Run `rfs init` first".into());
        }

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
            uploads: Mutex::new(HashMap::new()),
        })
    }

    pub fn register_upload(&self, id: &str, owner_id: &str) -> Result<bool, Box<dyn Error>> {
        let mut uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload state lock poisoned")?;
        uploads.retain(|_, state| {
            !state
                .completed_at
                .is_some_and(|at| at.elapsed().as_secs() > 900)
        });
        if uploads.contains_key(id) {
            return Ok(false);
        }
        uploads.insert(
            id.to_owned(),
            UploadState {
                owner_id: owner_id.to_owned(),
                cancelled: false,
                completed: false,
                completed_at: None,
            },
        );
        Ok(true)
    }

    pub fn cancel_upload(&self, id: &str, owner_id: &str) -> Result<Option<bool>, Box<dyn Error>> {
        let mut uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload state lock poisoned")?;
        let Some(state) = uploads.get_mut(id) else {
            uploads.insert(
                id.to_owned(),
                UploadState {
                    owner_id: owner_id.to_owned(),
                    cancelled: true,
                    completed: false,
                    completed_at: Some(Instant::now()),
                },
            );
            return Ok(Some(true));
        };
        if state.owner_id != owner_id {
            return Ok(None);
        }
        if state.completed {
            return Ok(Some(false));
        }
        state.cancelled = true;
        Ok(Some(true))
    }

    pub fn upload_cancelled(&self, id: &str, owner_id: &str) -> Result<bool, Box<dyn Error>> {
        let uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload state lock poisoned")?;
        Ok(uploads
            .get(id)
            .is_none_or(|state| state.owner_id != owner_id || state.cancelled))
    }

    pub fn commit_upload(
        &self,
        id: &str,
        owner_id: &str,
        commit: impl FnOnce() -> Result<(), Box<dyn Error>>,
    ) -> Result<bool, Box<dyn Error>> {
        let mut uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload state lock poisoned")?;
        let Some(state) = uploads.get_mut(id) else {
            return Ok(false);
        };
        if state.owner_id != owner_id || state.cancelled || state.completed {
            return Ok(false);
        }
        commit()?;
        state.completed = true;
        state.completed_at = Some(Instant::now());
        Ok(true)
    }

    pub fn remove_upload(&self, id: &str) {
        if let Ok(mut uploads) = self.uploads.lock()
            && uploads.get(id).is_some_and(|state| !state.completed)
        {
            uploads.remove(id);
        }
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
            owner_id: None,
            folder_id: None,
        };

        self.storage.save_file(&file, &self.crypto)?;
        if let Err(error) = self.metadata.save_file(&metadata) {
            let _ = self.storage.delete_file(&file.id);
            return Err(error);
        }

        Ok(metadata)
    }

    pub fn list_files(&self) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let mut files = self.metadata.list_files()?;

        files.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(files)
    }

    pub fn get_file_for_user(
        &self,
        user: &UserAccount,
        id: &str,
    ) -> Result<Option<FileMetadata>, Box<dyn std::error::Error>> {
        self.metadata.get_file_for_owner(id, &user.id)
    }

    pub fn list_files_for_user(
        &self,
        user: &UserAccount,
    ) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let mut files = self.metadata.list_files_for_owner(&user.id)?;
        files.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(files)
    }

    pub fn list_files_for_user_in_folder(
        &self,
        user: &UserAccount,
        folder_id: &str,
    ) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let mut files = self
            .metadata
            .list_files_for_owner_folder(&user.id, folder_id)?;
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

    pub fn delete_file_for_user(
        &self,
        user: &UserAccount,
        id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.get_file_for_user(user, id)?.is_none() {
            return Err("File not found".into());
        }
        self.delete_file(id)
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
    pub fn export_chunked_for_user(
        &self,
        user: &UserAccount,
        id: &str,
    ) -> Result<ChunkIterator, Box<dyn std::error::Error>> {
        if self.get_file_for_user(user, id)?.is_none() {
            return Err("File not found".into());
        }
        Ok(self.storage.stream_chunks(id, &self.user_crypto(user)?)?)
    }

    pub fn export_range_for_user(
        &self,
        user: &UserAccount,
        id: &str,
        byte_start: u64,
        byte_end: u64,
    ) -> Result<RangeChunkIterator, Box<dyn std::error::Error>> {
        if self.get_file_for_user(user, id)?.is_none() {
            return Err("File not found".into());
        }
        Ok(self
            .storage
            .stream_chunks_range(id, &self.user_crypto(user)?, byte_start, byte_end)?)
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
