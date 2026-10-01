use crate::{crypto::Crypto, domain::StoredFile};
use aes_gcm::{Nonce, aead::Aead};
use std::{
    collections::HashSet,
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

        // The end of the requested range is not necessarily the end of the file.
        let frame_len = self
            .frame_size
            .min(self.file_len.saturating_sub(self.offset)) as usize;
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

    /// Removes published blobs which have no metadata record after an interrupted upload.
    pub fn cleanup_orphaned_files(&self, known_ids: &HashSet<String>) -> io::Result<usize> {
        let mut removed = 0;
        for entry in fs::read_dir(&self.root_path)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            if !known_ids.contains(&id) {
                fs::remove_file(entry.path())?;
                removed += 1;
                println!("=== Removed orphaned blob: {id}");
            }
        }
        Ok(removed)
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
        let file = std::fs::File::create(path)?;
        set_file_permissions(&file)?;
        Ok(file)
    }

    /// Renames the temporary file to the final file after successful writing and saving of metadata.
    pub fn finalize_file(&self, id: &str) -> io::Result<()> {
        let tmp_path = self.tmp_file_path(id);
        let final_path = self.file_path(id);
        fs::rename(tmp_path, final_path)?;
        Ok(())
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

#[cfg(unix)]
fn set_file_permissions(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_file_permissions(_file: &fs::File) -> io::Result<()> {
    Ok(())
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
    fn ranges_in_large_files_stop_at_encrypted_frame_boundaries() {
        use crate::crypto::CHUNK_SIZE;
        let root = std::env::temp_dir().join(format!("rfs-range-test-{}", uuid::Uuid::new_v4()));
        let storage = Storage::new(root.join("blobs"), root.join("tmp")).unwrap();
        let crypto = Crypto::new(KEY).unwrap();
        let source: Vec<u8> = (0..2 * CHUNK_SIZE + 4096)
            .map(|index| (index % 251) as u8)
            .collect();
        let file = StoredFile {
            id: "large-file".into(),
            filename: "large.bin".into(),
            content: source.clone(),
        };
        storage.save_file(&file, &crypto).unwrap();
        for (start, end) in [
            (11, 4095),
            (CHUNK_SIZE + 17, CHUNK_SIZE + 1023),
            (CHUNK_SIZE - 100, CHUNK_SIZE + 100),
            (2 * CHUNK_SIZE - 100, 2 * CHUNK_SIZE + 100),
            (source.len() - 100, source.len() - 1),
        ] {
            let actual = storage
                .stream_chunks_range(&file.id, &crypto, start as u64, end as u64)
                .unwrap()
                .collect::<io::Result<Vec<_>>>()
                .unwrap()
                .concat();
            assert_eq!(actual, source[start..=end], "range {start}-{end}");
        }
        fs::remove_dir_all(root).unwrap();
    }

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
