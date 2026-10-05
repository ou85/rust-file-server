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

    pub fn create_folder(
        &self,
        owner_id: &str,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<FolderMetadata, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut folders = transaction.open_table(FOLDERS)?;
        let parent_id = parent_id
            .map(str::to_owned)
            .unwrap_or_else(|| root_folder_id(owner_id));
        {
            let parent = folders
                .get(parent_id.as_str())?
                .ok_or("Parent folder not found")?;
            let parent: FolderMetadata = serde_json::from_str(parent.value())?;
            if parent.owner_id != owner_id {
                return Err("Access denied".into());
            }
        }
        for entry in folders.iter()? {
            let (_, value) = entry?;
            let folder: FolderMetadata = serde_json::from_str(value.value())?;
            if folder.owner_id == owner_id
                && folder.parent_id.as_deref() == Some(parent_id.as_str())
                && folder.name == name
            {
                return Err("Folder already exists".into());
            }
        }
        let folder = FolderMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: owner_id.to_owned(),
            parent_id: Some(parent_id),
            name: name.to_owned(),
            created_at: now_secs(),
        };
        let json = serde_json::to_string(&folder)?;
        folders.insert(folder.id.as_str(), json.as_str())?;
        drop(folders);
        transaction.commit()?;
        Ok(folder)
    }

    pub fn rename_folder(
        &self,
        owner_id: &str,
        id: &str,
        name: &str,
    ) -> Result<Option<FolderMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut folders = transaction.open_table(FOLDERS)?;
        let Some(mut folder) = ({
            let value = folders.get(id)?;
            value.map(|value| serde_json::from_str::<FolderMetadata>(value.value()))
        })
        .transpose()?
        else {
            return Ok(None);
        };
        if folder.owner_id != owner_id {
            return Err("Access denied".into());
        }
        if folder.parent_id.is_none() {
            return Err("Root folder cannot be renamed".into());
        }
        for entry in folders.iter()? {
            let (key, value) = entry?;
            if key.value() == id {
                continue;
            }
            let other: FolderMetadata = serde_json::from_str(value.value())?;
            if other.owner_id == owner_id
                && other.parent_id == folder.parent_id
                && other.name == name
            {
                return Err("Folder already exists".into());
            }
        }
        folder.name = name.to_owned();
        let json = serde_json::to_string(&folder)?;
        folders.insert(id, json.as_str())?;
        drop(folders);
        transaction.commit()?;
        Ok(Some(folder))
    }

    pub fn delete_folder(
        &self,
        owner_id: &str,
        id: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let mut folders = transaction.open_table(FOLDERS)?;
        let Some(folder) = ({
            let value = folders.get(id)?;
            value.map(|value| serde_json::from_str::<FolderMetadata>(value.value()))
        })
        .transpose()?
        else {
            return Ok(false);
        };
        if folder.owner_id != owner_id {
            return Err("Access denied".into());
        }
        if folder.parent_id.is_none() {
            return Err("Root folder cannot be deleted".into());
        }
        if folders.iter()?.any(|entry| {
            entry
                .ok()
                .and_then(|(_, value)| serde_json::from_str::<FolderMetadata>(value.value()).ok())
                .is_some_and(|child| child.parent_id.as_deref() == Some(id))
        }) {
            return Err("Folder is not empty".into());
        }
        {
            let files = transaction.open_table(FILES)?;
            if files.iter()?.any(|entry| {
                entry
                    .ok()
                    .and_then(|(_, value)| serde_json::from_str::<FileMetadata>(value.value()).ok())
                    .is_some_and(|file| {
                        file.owner_id.as_deref() == Some(owner_id)
                            && file.folder_id.as_deref() == Some(id)
                    })
            }) {
                return Err("Folder is not empty".into());
            }
        }
        folders.remove(id)?;
        drop(folders);
        transaction.commit()?;
        Ok(true)
    }

    pub fn delete_folder_recursive(
        &self,
        owner_id: &str,
        id: &str,
    ) -> Result<Option<Vec<FileMetadata>>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        let folders = transaction.open_table(FOLDERS)?;
        let Some(folder) = ({
            let value = folders.get(id)?;
            value.map(|value| serde_json::from_str::<FolderMetadata>(value.value()))
        })
        .transpose()?
        else {
            return Ok(None);
        };
        if folder.owner_id != owner_id {
            return Err("Access denied".into());
        }
        if folder.parent_id.is_none() {
            return Err("Root folder cannot be deleted".into());
        }

        let mut folder_ids = vec![id.to_owned()];
        let mut index = 0;
        while index < folder_ids.len() {
            let parent = folder_ids[index].clone();
            for entry in folders.iter()? {
                let (_, value) = entry?;
                let child: FolderMetadata = serde_json::from_str(value.value())?;
                if child.owner_id == owner_id && child.parent_id.as_deref() == Some(parent.as_str())
                {
                    folder_ids.push(child.id);
                }
            }
            index += 1;
        }
        drop(folders);

        let mut files_to_delete = Vec::new();
        {
            let files = transaction.open_table(FILES)?;
            for entry in files.iter()? {
                let (key, value) = entry?;
                let file: FileMetadata = serde_json::from_str(value.value())?;
                if file.owner_id.as_deref() == Some(owner_id)
                    && file
                        .folder_id
                        .as_deref()
                        .is_some_and(|folder_id| folder_ids.iter().any(|id| id == folder_id))
                {
                    files_to_delete.push((key.value().to_owned(), file));
                }
            }
        }
        {
            let mut files = transaction.open_table(FILES)?;
            for (file_id, _) in &files_to_delete {
                files.remove(file_id.as_str())?;
            }
        }
        {
            let mut folders = transaction.open_table(FOLDERS)?;
            for folder_id in &folder_ids {
                folders.remove(folder_id.as_str())?;
            }
        }
        transaction.commit()?;
        Ok(Some(
            files_to_delete.into_iter().map(|(_, file)| file).collect(),
        ))
    }

    pub fn list_files_for_owner_folder(
        &self,
        owner_id: &str,
        folder_id: &str,
    ) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        Ok(self
            .list_files_for_owner(owner_id)?
            .into_iter()
            .filter(|file| file.folder_id.as_deref() == Some(folder_id))
            .collect())
    }

    pub fn file_name_exists(
        &self,
        owner_id: &str,
        folder_id: &str,
        filename: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(self
            .list_files_for_owner_folder(owner_id, folder_id)?
            .into_iter()
            .any(|file| file.filename == filename))
    }

    pub fn move_files_for_owner(
        &self,
        owner_id: &str,
        ids: &[String],
        folder_id: &str,
    ) -> Result<Vec<FileMetadata>, Box<dyn std::error::Error>> {
        let transaction = self.db.begin_write()?;
        {
            let folders = transaction.open_table(FOLDERS)?;
            let Some(value) = folders.get(folder_id)? else {
                return Err("Destination folder not found".into());
            };
            let folder: FolderMetadata = serde_json::from_str(value.value())?;
            if folder.owner_id != owner_id {
                return Err("Access denied".into());
            }
        }

        let mut candidates = Vec::with_capacity(ids.len());
        {
            let files = transaction.open_table(FILES)?;
            for id in ids {
                let file: FileMetadata = {
                    let Some(value) = files.get(id.as_str())? else {
                        return Err("File not found".into());
                    };
                    serde_json::from_str(value.value())?
                };
                if file.owner_id.as_deref() != Some(owner_id) {
                    return Err("Access denied".into());
                }
                candidates.push(file);
            }
            if candidates.iter().enumerate().any(|(index, file)| {
                candidates
                    .iter()
                    .skip(index + 1)
                    .any(|other| other.filename == file.filename)
            }) {
                return Err(
                    "A file with this name already exists in the destination folder".into(),
                );
            }
            for entry in files.iter()? {
                let (key, value) = entry?;
                let other: FileMetadata = serde_json::from_str(value.value())?;
                if other.owner_id.as_deref() == Some(owner_id)
                    && other.folder_id.as_deref() == Some(folder_id)
                    && candidates.iter().any(|file| {
                        file.filename == other.filename
                            && !ids.iter().any(|id| id.as_str() == key.value())
                    })
                {
                    return Err(
                        "A file with this name already exists in the destination folder".into(),
                    );
                }
            }
        }
        let mut moved = Vec::with_capacity(candidates.len());
        {
            let mut files = transaction.open_table(FILES)?;
            for mut file in candidates {
                file.folder_id = Some(folder_id.to_owned());
                let json = serde_json::to_string(&file)?;
                files.insert(file.id.as_str(), json.as_str())?;
                moved.push(file);
            }
        }
        transaction.commit()?;
        Ok(moved)
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
