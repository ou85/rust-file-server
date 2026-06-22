use crate::{
    crypto::Crypto,
    models::StoredFile,
};
use std::{fs, io, path::Path};

pub struct Storage {
    root_path: String,
}

impl Storage {
    pub fn new(root_path: String) -> io::Result<Self> {
        fs::create_dir_all(&root_path)?;
        println!("=== Storage initialized");
        Ok(Self { root_path })
    }

    fn file_path(&self, id: &str) -> String {
        format!("{}/{}", self.root_path, id)
    }

    /// Saves the file in chunks - each chunk is encrypted separately
    pub fn save_file(&self, file: &StoredFile, crypto: &Crypto) -> io::Result<String> {
        let path = self.file_path(&file.id);
        let encrypted = crypto.encrypt_chunked(&file.content);
        fs::write(path, encrypted)?;
        Ok(file.id.clone())
    }

    /// Reads an encrypted file and returns an iterator over the decrypted chunks (one chunk ~8MB )
    pub fn stream_chunks(&self, id: &str, crypto: &Crypto) -> io::Result<ChunkIterator> {
        let path = self.file_path(id);
        let data = fs::read(path)?;

        if data.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }

        let chunk_size = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;

        Ok(ChunkIterator {
            data,
            offset: 4,
            chunk_size,
            crypto: crypto.clone(), 
        })
    }

    /// For small files (open/preview) - reads everything in Vec<u8>
    pub fn export_to_bytes(&self, id: &str, crypto: &Crypto) -> io::Result<Vec<u8>> {
        let mut result = Vec::new();
        for chunk in self.stream_chunks(id, crypto)? {
            result.extend_from_slice(&chunk?);
        }
        Ok(result)
    }

    pub fn delete_file(&self, id: &str) -> io::Result<()> {
        fs::remove_file(self.file_path(id))?;
        Ok(())
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
}

/// Iterator over decrypted chunks.
/// Stores data of one chunk at a time in RAM
pub struct ChunkIterator {
    data: Vec<u8>,     
    offset: usize,    
    chunk_size: usize,
    crypto: Crypto,
}

impl<'a> Iterator for ChunkIterator {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.data.len() {
            return None;
        }

        // chunk ciphertext = plaintext + 16 bytes GCM tag
        let ct_size = self.chunk_size + 16;
        // nonce (12) + ciphertext
        let frame_size = 12 + ct_size;
        let end = (self.offset + frame_size).min(self.data.len());
        let frame = &self.data[self.offset..end];

        if frame.len() < 12 {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupt chunk",
            )));
        }

        let nonce_bytes: [u8; 12] = frame[..12].try_into().unwrap();
        let ciphertext = &frame[12..];

        match self.crypto.decrypt(&nonce_bytes, ciphertext) {
            Ok(plain) => {
                self.offset += frame.len();
                Some(Ok(plain))
            }
            Err(_) => Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decryption failed",
            ))),
        }
    }
}

pub fn guess_mime(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .to_string()
}
