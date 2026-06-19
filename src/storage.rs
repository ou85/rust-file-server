use crate::{crypto::Crypto, models::StoredFile};
use std::time::Instant;
use std::{fs, io, path::Path};

pub struct Storage {
    root_path: String,
}

impl Storage {
    pub fn new(root_path: String) -> io::Result<Self> {
        fs::create_dir_all(&root_path)?;

        // println!("=== Storage initialized: {}", root_path);
        println!("=== Storage initialized");

        Ok(Self { root_path })
    }

    // File format:
    //
    // [12-byte nonce]
    // [ciphertext + 16-byte GCM tag]
    //
    // No debugger ----------------------------------------
    // pub fn save_file(&self, file: &StoredFile, crypto: &Crypto) -> io::Result<String> {
    //     let path = format!("{}/{}", self.root_path, file.id);

    //     let (nonce, encrypted) = crypto.encrypt(&file.content);

    //     let mut output = Vec::new();

    //     output.extend_from_slice(&nonce);
    //     output.extend_from_slice(&encrypted);

    //     fs::write(path, output)?;

    //     Ok(file.id.clone())
    // }
    // ------------------------------------
    //
    // With debugger ------------------------------------
    pub fn save_file(&self, file: &StoredFile, crypto: &Crypto) -> io::Result<String> {
        let path = format!("{}/{}", self.root_path, file.id);

        let start = Instant::now();
        let (nonce, encrypted) = crypto.encrypt(&file.content);
        println!("=== encrypt: {:?}", start.elapsed());

        let start = Instant::now();

        let mut output = Vec::new();
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&encrypted);

        println!("=== build output: {:?}", start.elapsed());

        let start = Instant::now();
        fs::write(path, output)?;
        println!("=== write file: {:?}", start.elapsed());

        Ok(file.id.clone())
    }
    // ---------------------------------------------------------

    pub fn read_file(&self, id: &str, crypto: &Crypto) -> io::Result<Vec<u8>> {
        let path = format!("{}/{}", self.root_path, id);

        let data = fs::read(path)?;

        if data.len() < 12 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }

        let nonce_bytes: [u8; 12] = data[..12].try_into().unwrap();
        let encrypted = &data[12..];
        let decrypted = crypto.decrypt(&nonce_bytes, encrypted);

        Ok(decrypted)
    }

    pub fn import_file(path: &str) -> io::Result<StoredFile> {
        let content = fs::read(path)?;

        let filename = Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        Ok(StoredFile {
            id: crate::id::id_16(),
            filename,
            content,
        })
    }

    /// Reads an encrypted file from storage, decrypts it,
    /// and writes the original content to the destination path.        
    // pub fn export_file(&self, id: &str, destination: &str, crypto: &Crypto) -> io::Result<()> {
    //     let content = self.read_file(id, crypto)?;

    //     fs::write(destination, content)?;

    //     Ok(())
    // }

    pub fn delete_file(&self, id: &str) -> io::Result<()> {
        let path = format!("{}/{}", self.root_path, id);
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn export_to_bytes(&self, id: &str, crypto: &Crypto) -> io::Result<Vec<u8>> {
        self.read_file(id, crypto)
    }
}

pub fn guess_mime(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .to_string()
}
