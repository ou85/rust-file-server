use crate::{
    config::Config,
    domain::{FileMetadata, FolderMetadata, UserAccount},
};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

const FILES: TableDefinition<&str, &str> = TableDefinition::new("files");
const USERS: TableDefinition<&str, &str> = TableDefinition::new("users");
const USERNAMES: TableDefinition<&str, &str> = TableDefinition::new("usernames");
const FOLDERS: TableDefinition<&str, &str> = TableDefinition::new("folders");

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
            let _ = transaction.open_table(FOLDERS)?;
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
        let root = FolderMetadata {
            id: root_folder_id(&user.id),
            owner_id: user.id.clone(),
            parent_id: None,
            name: String::new(),
            created_at: now_secs(),
        };
        let root_json = serde_json::to_string(&root)?;
        {
            let mut folders = transaction.open_table(FOLDERS)?;
            folders.insert(root.id.as_str(), root_json.as_str())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn root_folder(
        &self,
        owner_id: &str,
    ) -> Result<Option<FolderMetadata>, Box<dyn std::error::Error>> {
        self.get_folder(&root_folder_id(owner_id))
    }

    pub fn ensure_root_folder(
        &self,
        owner_id: &str,
    ) -> Result<FolderMetadata, Box<dyn std::error::Error>> {
        if let Some(folder) = self.root_folder(owner_id)? {
            return Ok(folder);
        }
        let root = FolderMetadata {
            id: root_folder_id(owner_id),
            owner_id: owner_id.to_owned(),
            parent_id: None,
            name: String::new(),
            created_at: now_secs(),
        };
        self.save_folder(&root)?;
        Ok(root)
    }

    pub fn save_folder(&self, folder: &FolderMetadata) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(folder)?;
        let transaction = self.db.begin_write()?;
        {
            let mut table = transaction.open_table(FOLDERS)?;
            table.insert(folder.id.as_str(), json.as_str())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_folder(
        &self,
        id: &str,
    ) -> Result<Option<FolderMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(FOLDERS)?;
        table
            .get(id)?
            .map(|value| serde_json::from_str(value.value()))
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_child_folders(
        &self,
        owner_id: &str,
        parent_id: Option<&str>,
    ) -> Result<Vec<FolderMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_read()?;
        let table = transaction.open_table(FOLDERS)?;
        let mut folders = Vec::new();
        for entry in table.iter()? {
            let (_, value) = entry?;
            let folder: FolderMetadata = serde_json::from_str(value.value())?;
            if folder.owner_id == owner_id && folder.parent_id.as_deref() == parent_id {
                folders.push(folder);
            }
        }
        folders.sort_by_cached_key(|folder| folder.name.to_lowercase());
        Ok(folders)
    }

    pub fn folder_name_exists(
        &self,
        owner_id: &str,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(self
            .list_child_folders(owner_id, parent_id)?
            .into_iter()
            .any(|folder| folder.name == name))
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

    pub fn change_password(
        &self,
        id: &str,
        password_hash: &str,
    ) -> Result<Option<UserAccount>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut user = {
            let users = transaction.open_table(USERS)?;
            let Some(value) = users.get(id)? else {
                return Ok(None);
            };
            serde_json::from_str::<UserAccount>(value.value())?
        };
        user.password_hash = password_hash.to_owned();
        user.password_change_required = false;
        user.auth_version = user.auth_version.saturating_add(1);
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

    pub fn get_file_for_owner(
        &self,
        id: &str,
        owner_id: &str,
    ) -> Result<Option<FileMetadata>, Box<dyn std::error::Error>> {
        Ok(self
            .get_file(id)?
            .filter(|file| file.owner_id.as_deref() == Some(owner_id)))
    }

    pub fn list_files_for_owner(
        &self,
        owner_id: &str,
    ) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        Ok(self
            .list_files()?
            .into_iter()
            .filter(|file| file.owner_id.as_deref() == Some(owner_id))
            .collect())
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

pub fn root_folder_id(owner_id: &str) -> String {
    format!("root-{owner_id}")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
