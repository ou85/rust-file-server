use crate::{
    config::Config,
    crypto::Crypto,
    database::MetadataStore,
    models::FileMetadata,
    storage::{ChunkIterator, RangeChunkIterator, Storage},
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
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::new();

        ensure_dir_exists(&config.storage_path)?;
        ensure_dir_exists(&config.database_path)?;

        let storage = Arc::new(Storage::new(config.storage_path.clone())?);

        let database = if config.database_path == config.storage_path {
            Arc::clone(&storage)
        } else {
            Arc::new(Storage::new(config.database_path.clone())?)
        };

        let metadata = MetadataStore::new(&config)?;

        let crypto = Crypto::new(&config.encryption_key);

        println!("=== Storage initialized at: {}", config.storage_path);
        if !Arc::ptr_eq(&storage, &database) {
            println!("=== Database initialized at: {}", config.database_path);
        } else {
            println!("=== Database uses same path as storage");
        }

        Ok(Self {
            config,
            storage,
            // database,
            metadata,
            crypto,
        })
    }

    pub fn print_banner(addr: &str, storage_path: &str, db_path: &str) {
        const BLUE: &str = "\x1b[36m";
        const RESET: &str = "\x1b[0m";

        println!("──────────────────────────────");
        println!("rust-file-server v{}", env!("CARGO_PKG_VERSION"));
        println!("──────────────────────────────");
        println!("✓ Storage:   {}", storage_path);
        println!("✓ Database:  {}", db_path);
        // println!("✓ Database:  ready");
        println!();
        println!("→ Server online");
        println!("→ Listening on {}http://{}{}", BLUE, addr, RESET);
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

        self.metadata.save_file(&metadata)?;

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
        self.storage.delete_file(id)?;
        self.metadata.delete_file(id)?;

        Ok(())
    }

    pub fn demo(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let metadata = self.import_file(path)?;

        println!("=== Imported: {} ({})", metadata.filename, metadata.id);

        for file in self.list_files()? {
            println!("{} | {} | {} bytes", file.id, file.filename, file.size,);
        }

        Ok(())
    }

    /// For preview (open_file) - small files, everything in memory
    pub fn export_to_bytes(&self, id: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(self.storage.export_to_bytes(id, &self.crypto)?)
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
        println!("Created directory: {}", p.display());
    }
    Ok(())
}
