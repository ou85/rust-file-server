use crate::{config::Config, crypto::Crypto, database::MetadataStore, storage::Storage};

use crate::models::FileMetadata;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct App {
    pub config: Config,
    pub storage: Storage,
    pub metadata: MetadataStore,
    pub crypto: Crypto,
}

impl App {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::new();

        let storage = Storage::new(config.storage_path.clone())?;

        let metadata = MetadataStore::new(&config)?;

        let crypto = Crypto::new(&config.encryption_key);

        Ok(Self {
            config,
            storage,
            metadata,
            crypto,
        })
    }

    pub fn print_banner(addr: &str, storage_path: &str) {
        const BLUE: &str = "\x1b[36m";
        const RESET: &str = "\x1b[0m";

        println!("──────────────────────────────");
        println!("rust-file-server v{}", env!("CARGO_PKG_VERSION"));
        println!("──────────────────────────────");
        println!("✓ Storage:   {}", storage_path);
        println!("✓ Database:  ready");
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

    pub fn export_file(
        &self,
        id: &str,
        destination: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.storage.export_file(id, destination, &self.crypto)?;

        Ok(())
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

    pub fn import_bytes(
        &self,
        filename: &str,
        content: Vec<u8>,
    ) -> Result<FileMetadata, Box<dyn std::error::Error>> {
        let file = crate::models::StoredFile {
            id: crate::id::id_16(),
            filename: filename.to_string(),
            content,
        };

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
}
