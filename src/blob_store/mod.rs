use crate::{crypto::Crypto, domain::StoredFile};
use aes_gcm::{Nonce, aead::Aead};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Iterator over decrypted chunks for a byte range.
/// Skips chunks before the range, decrypts only what's needed.
pub struct RangeChunkIterator {
    file: fs::File,
    offset: u64,
    file_len: u64,
    frame_size: u64,
    cipher: aes_gcm::Aes256Gcm,
    current_chunk: u64,
    last_chunk: u64,
    skip_bytes: usize,
    remaining: usize,
}

impl Iterator for RangeChunkIterator {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_chunk > self.last_chunk || self.remaining == 0 {
            return None;
        }

        let frame_len = if self.current_chunk == self.last_chunk {
            self.file_len.saturating_sub(self.offset) as usize
        } else {
            self.frame_size as usize
        };
        if frame_len < 12 {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupt chunk",
            )));
        }

        let mut frame = vec![0u8; frame_len];
        if let Err(error) = self.file.read_exact(&mut frame) {
            return Some(Err(error));
        }
        let nonce_bytes: [u8; 12] = frame[..12].try_into().unwrap();
        let ciphertext = &frame[12..];

        let plain = match self.cipher.decrypt(
            &Nonce::try_from(nonce_bytes.as_slice()).unwrap(),
            ciphertext,
        ) {
            Ok(p) => p,
            Err(_) => {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "decryption failed",
                )));
            }
        };

        self.offset += frame_len as u64;
        self.current_chunk += 1;

        let start = self.skip_bytes;
        self.skip_bytes = 0;

        let available = &plain[start..];
        let take = available.len().min(self.remaining);
        self.remaining -= take;

        Some(Ok(available[..take].to_vec()))
    }
}
pub struct Storage {
    root_path: PathBuf,
    tmp_path: PathBuf,
}

impl Storage {
    pub fn new(root_path: PathBuf, tmp_path: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root_path)?;
        fs::create_dir_all(&tmp_path)?;
        println!("\n=== Storage initialized");
        Ok(Self {
            root_path,
            tmp_path,
        })
    }

    fn file_path(&self, id: &str) -> PathBuf {
        self.root_path.join(id)
    }

    fn tmp_file_path(&self, id: &str) -> PathBuf {
        self.tmp_path.join(format!("{id}.partial"))
    }

    /// Saves the file in chunks - each chunk is encrypted separately
    pub fn save_file(&self, file: &StoredFile, crypto: &Crypto) -> io::Result<String> {
        let path = self.file_path(&file.id);
        let encrypted = crypto.encrypt_chunked(&file.id, &file.content)?;
        fs::write(path, encrypted)?;
        Ok(file.id.clone())
    }

    /// Reads an encrypted file and returns an iterator over the decrypted chunks (one chunk ~8MB )
    pub fn stream_chunks(&self, id: &str, crypto: &Crypto) -> io::Result<ChunkIterator> {
        let path = self.file_path(id);
        let mut file = fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut header = [0u8; 25];
        let bytes_read = file.read(&mut header)?;
        let (offset, chunk_size, cipher) = crypto.parse_cipher(id, &header[..bytes_read])?;
        if file_len < offset as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated file header",
            ));
        }
        file.seek(SeekFrom::Start(offset as u64))?;

        Ok(ChunkIterator {
            file,
            remaining_encrypted: file_len - offset as u64,
            chunk_size,
            cipher,
        })
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
            id: crate::domain::id::id_16(),
            filename,
            content,
        })
    }

    pub fn stream_chunks_range(
        &self,
        id: &str,
        crypto: &Crypto,
        byte_start: u64,
        byte_end: u64,
    ) -> io::Result<RangeChunkIterator> {
        let path = self.file_path(id);
        let mut file = fs::File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut header = [0u8; 25];
        let bytes_read = file.read(&mut header)?;
        let (header_len, chunk_size, cipher) = crypto.parse_cipher(id, &header[..bytes_read])?;
        let frame_size = (12 + chunk_size + 16) as u64;

        let first_chunk = byte_start / chunk_size as u64;
        let last_chunk = byte_end / chunk_size as u64;
        let skip_bytes = (byte_start % chunk_size as u64) as usize;
        let remaining = (byte_end - byte_start + 1) as usize;
        let offset = header_len as u64 + first_chunk * frame_size;
        if offset >= file_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "range points outside encrypted data",
            ));
        }
        file.seek(SeekFrom::Start(offset))?;

        Ok(RangeChunkIterator {
            file,
            offset,
            file_len,
            frame_size,
            cipher,
            current_chunk: first_chunk,
            last_chunk,
            skip_bytes,
            remaining,
        })
    }

    pub fn create_file_writer(&self, id: &str) -> io::Result<std::fs::File> {
        let path = self.tmp_file_path(id);
        Ok(std::fs::File::create(path)?)
    }

    /// Renames the temporary file to the final file after successful writing and saving of metadata.
    pub fn finalize_file(&self, id: &str) -> io::Result<()> {
        let tmp_path = self.tmp_file_path(id);
        let final_path = self.file_path(id);
        fs::rename(tmp_path, final_path)?;
        Ok(())
    }

    /// Deletes the .tmp file in case of an error (rollback).
    pub fn cleanup_tmp_file(&self, id: &str) {
        let tmp_path = self.tmp_file_path(id);
        let _ = fs::remove_file(tmp_path);
    }

    /// Deletes ALL .tmp files (called at server startup).
    pub fn cleanup_all_tmp_files(&self) -> io::Result<usize> {
        let mut removed = 0;

        for entry in fs::read_dir(&self.tmp_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if name.ends_with(".partial") {
                if fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                    println!("=== Removed orphaned tmp file: {}", name);
                }
            }
        }

        Ok(removed)
    }

    /// Deletes .tmp files older than max_age seconds.
    pub fn cleanup_stale_tmp_files(&self, max_age_secs: u64) -> io::Result<usize> {
        let mut removed = 0;
        let max_age = std::time::Duration::from_secs(max_age_secs);

        for entry in fs::read_dir(&self.tmp_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if name.ends_with(".partial") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(elapsed) = modified.elapsed() {
                            if elapsed > max_age {
                                if fs::remove_file(entry.path()).is_ok() {
                                    removed += 1;
                                    println!("=== Removed stale tmp file: {}", name);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(removed)
    }
}

/// Iterator over decrypted chunks.
/// Stores data of one chunk at a time in RAM
pub struct ChunkIterator {
    file: fs::File,
    remaining_encrypted: u64,
    chunk_size: usize,
    cipher: aes_gcm::Aes256Gcm,
}

impl Iterator for ChunkIterator {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_encrypted == 0 {
            return None;
        }

        // chunk ciphertext = plaintext + 16 bytes GCM tag
        let ct_size = self.chunk_size + 16;
        // nonce (12) + ciphertext
        let frame_size = 12 + ct_size;
        let frame_len = (frame_size as u64).min(self.remaining_encrypted) as usize;
        let mut frame = vec![0u8; frame_len];
        if let Err(error) = self.file.read_exact(&mut frame) {
            return Some(Err(error));
        }

        if frame.len() < 12 {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupt chunk",
            )));
        }

        let nonce_bytes: [u8; 12] = frame[..12].try_into().unwrap();
        let ciphertext = &frame[12..];

        match self.cipher.decrypt(
            &Nonce::try_from(nonce_bytes.as_slice()).unwrap(),
            ciphertext,
        ) {
            Ok(plain) => {
                self.remaining_encrypted -= frame_len as u64;
                Some(Ok(plain))
            }
            Err(_) => Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decryption failed",
            ))),
        }
    }
}

// ------------------ Public Methods ------------------

pub fn guess_mime(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn streams_full_file_and_requested_range() {
        let root = std::env::temp_dir().join(format!("rfs-stream-test-{}", uuid::Uuid::new_v4()));
        let storage = Storage::new(root.join("blobs"), root.join("tmp")).unwrap();
        let crypto = Crypto::new(KEY).unwrap();
        let source = b"streaming encrypted content".to_vec();
        let file = StoredFile {
            id: "test-file".into(),
            filename: "test.txt".into(),
            content: source.clone(),
        };
        storage.save_file(&file, &crypto).unwrap();

        let all: Vec<u8> = storage
            .stream_chunks(&file.id, &crypto)
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap()
            .concat();
        let range: Vec<u8> = storage
            .stream_chunks_range(&file.id, &crypto, 3, 11)
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap()
            .concat();
        assert_eq!(all, source);
        assert_eq!(range, source[3..=11]);
        fs::remove_dir_all(root).unwrap();
    }
}
