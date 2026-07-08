use crate::{config::Config, models::FileMetadata};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const FILES: TableDefinition<&str, &str> = TableDefinition::new("files");

pub struct MetadataStore {
    db: Database,
}

impl MetadataStore {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let path = format!("{}/metadata.redb", config.database_path);

        let db = Database::create(path)?;

        let write_txn = db.begin_write()?;

        {
            let _table = write_txn.open_table(FILES)?;
        }

        println!("=== Database initialized");

        Ok(Self { db })
    }

    pub fn save_file(&self, metadata: &FileMetadata) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(metadata)?;
        let write_txn = self.db.begin_write()?;

        {
            let mut table = write_txn.open_table(FILES)?;

            table.insert(metadata.id.as_str(), json.as_str())?;
        }

        write_txn.commit()?;

        Ok(())
    }

    pub fn get_file(&self, id: &str) -> Result<Option<FileMetadata>, Box<dyn std::error::Error>> {
        let read_txn = self.db.begin_read()?;

        let table = read_txn.open_table(FILES)?;

        if let Some(value) = table.get(id)? {
            let metadata: FileMetadata = serde_json::from_str(value.value())?;
            return Ok(Some(metadata));
        }

        Ok(None)
    }

    pub fn list_files(&self) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let read_txn = self.db.begin_read()?;

        let table = read_txn.open_table(FILES)?;

        let mut files = Vec::new();

        for entry in table.iter()? {
            let (_, value) = entry?;

            let metadata: FileMetadata = serde_json::from_str(value.value())?;

            files.push(metadata);
        }

        Ok(files)
    }

    pub fn delete_file(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let write_txn = self.db.begin_write()?;

        {
            let mut table = write_txn.open_table(FILES)?;

            table.remove(id)?;
        }

        write_txn.commit()?;

        Ok(())
    }
}
