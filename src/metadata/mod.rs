use crate::{
    config::Config,
    domain::{AuthRecord, FileMetadata},
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const FILES: TableDefinition<&str, &str> = TableDefinition::new("files");
const AUTH: TableDefinition<&str, &str> = TableDefinition::new("auth");
const AUTH_KEY: &str = "user";

pub struct MetadataStore {
    db: Database,
}

impl MetadataStore {
    #[cfg(test)]
    pub(crate) fn insert_invalid_record(&self, id: &str) {
        let transaction = self.db.begin_write().unwrap();
        {
            let mut table = transaction.open_table(FILES).unwrap();
            table.insert(id, "not valid JSON").unwrap();
        }
        transaction.commit().unwrap();
    }

    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::create(&config.metadata_path)?;

        let write_txn = db.begin_write()?;

        {
            let _table = write_txn.open_table(FILES)?;
            let _auth = write_txn.open_table(AUTH)?;
        }
        write_txn.commit()?;

        tracing::info!("Database initialized");

        Ok(Self { db })
    }

    pub fn auth_record(&self) -> Result<Option<AuthRecord>, Box<dyn std::error::Error>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(AUTH)?;
        table
            .get(AUTH_KEY)?
            .map(|value| serde_json::from_str(value.value()))
            .transpose()
            .map_err(Into::into)
    }

    pub fn save_auth_record(&self, record: &AuthRecord) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(record)?;
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(AUTH)?;
            table.insert(AUTH_KEY, json.as_str())?;
        }
        txn.commit()?;
        Ok(())
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
