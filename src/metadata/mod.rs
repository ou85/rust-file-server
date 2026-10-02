use crate::{
    config::Config,
    domain::{FileMetadata, UserAccount},
};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

const FILES: TableDefinition<&str, &str> = TableDefinition::new("files");
const USERS: TableDefinition<&str, &str> = TableDefinition::new("users");
const USERNAMES: TableDefinition<&str, &str> = TableDefinition::new("usernames");

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
        let transaction = db.begin_write()?;
        {
            let _ = transaction.open_table(FILES)?;
            let _ = transaction.open_table(USERS)?;
            let _ = transaction.open_table(USERNAMES)?;
        }
        transaction.commit()?;
        tracing::info!("Database initialized");
        Ok(Self { db })
    }

    pub fn user_count(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(USERS)?;
        Ok(table.len()? as usize)
    }

    pub fn create_user(&self, user: &UserAccount) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(user)?;
        let transaction = self.db.begin_write()?;
        {
            let mut names = transaction.open_table(USERNAMES)?;
            if names.get(user.username.as_str())?.is_some() {
                return Err("Username already exists".into());
            }
            names.insert(user.username.as_str(), user.id.as_str())?;
        }
        {
            let mut users = transaction.open_table(USERS)?;
            users.insert(user.id.as_str(), json.as_str())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_user(&self, id: &str) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(USERS)?;
        table
            .get(id)?
            .map(|value| serde_json::from_str(value.value()))
            .transpose()
            .map_err(Into::into)
    }

    pub fn get_user_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let id = {
            let names = transaction.open_table(USERNAMES)?;
            names.get(username)?.map(|value| value.value().to_owned())
        };
        let Some(id) = id else {
            return Ok(None);
        };
        let users = transaction.open_table(USERS)?;
        users
            .get(id.as_str())?
            .map(|value| serde_json::from_str(value.value()))
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_users(&self) -> Result<Vec<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(USERS)?;
        let mut users = Vec::new();
        for entry in table.iter()? {
            let (_, value) = entry?;
            users.push(serde_json::from_str(value.value())?);
        }
        users.sort_by(|left: &UserAccount, right: &UserAccount| left.username.cmp(&right.username));
        Ok(users)
    }

    pub fn rename_user(
        &self,
        id: &str,
        username: &str,
    ) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut user = {
            let users = transaction.open_table(USERS)?;
            let Some(value) = users.get(id)? else {
                return Ok(None);
            };
            serde_json::from_str::<UserAccount>(value.value())?
        };
        if user.username == username {
            return Ok(Some(user));
        }
        {
            let names = transaction.open_table(USERNAMES)?;
            if names.get(username)?.is_some() {
                return Err("Username already exists".into());
            }
        }
        let old_username = std::mem::replace(&mut user.username, username.to_owned());
        let json = serde_json::to_string(&user)?;
        {
            let mut names = transaction.open_table(USERNAMES)?;
            names.remove(old_username.as_str())?;
            names.insert(username, id)?;
        }
        {
            let mut users = transaction.open_table(USERS)?;
            users.insert(id, json.as_str())?;
        }
        transaction.commit()?;
        Ok(Some(user))
    }

    pub fn require_password_change(
        &self,
        id: &str,
    ) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut user = {
            let users = transaction.open_table(USERS)?;
            let Some(value) = users.get(id)? else {
                return Ok(None);
            };
            serde_json::from_str::<UserAccount>(value.value())?
        };
        user.password_change_required = true;
        let json = serde_json::to_string(&user)?;
        {
            let mut users = transaction.open_table(USERS)?;
            users.insert(id, json.as_str())?;
        }
        transaction.commit()?;
        Ok(Some(user))
    }

    pub fn legacy_file_count(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        Ok(transaction.open_table(FILES)?.len()? as usize)
    }

    pub fn save_file(&self, metadata: &FileMetadata) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(metadata)?;
        let transaction = self.db.begin_write()?;
        {
            let mut table = transaction.open_table(FILES)?;
            table.insert(metadata.id.as_str(), json.as_str())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_file(&self, id: &str) -> Result<Option<FileMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(FILES)?;
        table
            .get(id)?
            .map(|value| serde_json::from_str(value.value()))
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_files(&self) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(FILES)?;
        let mut files = Vec::new();
        for entry in table.iter()? {
            let (_, value) = entry?;
            files.push(serde_json::from_str(value.value())?);
        }
        Ok(files)
    }

    pub fn delete_file(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        {
            let mut table = transaction.open_table(FILES)?;
            table.remove(id)?;
        }
        transaction.commit()?;
        Ok(())
    }
}
