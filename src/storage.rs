use crate::{crypto::Crypto, models::StoredFile};
use std::{fs, io, path::Path};

/// Iterator over decrypted chunks for a byte range.
/// Skips chunks before the range, decrypts only what's needed.
pub struct RangeChunkIterator {
    data: Vec<u8>,
    offset: usize,
    frame_size: usize,
    crypto: Crypto,
    current_chunk: usize,
    last_chunk: usize,
    skip_bytes: usize,
    remaining: usize,
}

impl Iterator for RangeChunkIterator {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_chunk > self.last_chunk || self.remaining == 0 {
            return None;
        }

        let end = (self.offset + self.frame_size).min(self.data.len());
        let frame = &self.data[self.offset..end];

        if frame.len() < 12 {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupt chunk",
            )));
        }

        let nonce_bytes: [u8; 12] = frame[..12].try_into().unwrap();
        let ciphertext = &frame[12..];

        let plain = match self.crypto.decrypt(&nonce_bytes, ciphertext) {
            Ok(p) => p,
            Err(_) => {
                return Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "decryption failed",
                )));
            }
        };

        self.offset += frame.len();
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
    root_path: String,
}

impl Storage {
    pub fn new(root_path: String) -> io::Result<Self> {
        fs::create_dir_all(&root_path)?;
        println!("\n=== Storage initialized");
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

    pub fn stream_chunks_range(
        &self,
        id: &str,
        crypto: &Crypto,
        byte_start: u64,
        byte_end: u64,
    ) -> io::Result<RangeChunkIterator> {
        let path = self.file_path(id);
        let data = fs::read(path)?;

        if data.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }

        let chunk_size = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        let frame_size = 12 + chunk_size + 16; // nonce + ciphertext + GCM tag

        let first_chunk = byte_start as usize / chunk_size;
        let last_chunk = byte_end as usize / chunk_size;
        let skip_bytes = byte_start as usize % chunk_size;
        let remaining = (byte_end - byte_start + 1) as usize;

        // Fast-forward directly to the required chunk, skipping the previous ones
        let offset = 4 + first_chunk * frame_size;

        Ok(RangeChunkIterator {
            data,
            offset,
            frame_size,
            crypto: crypto.clone(),
            current_chunk: first_chunk,
            last_chunk,
            skip_bytes,
            remaining,
        })
    }

    pub fn create_file_writer(&self, id: &str) -> io::Result<std::fs::File> {
        let path = self.file_path(&format!("{}.tmp", id));
        Ok(std::fs::File::create(path)?)
    }

    /// Renames the temporary file to the final file after successful writing and saving of metadata.
    pub fn finalize_file(&self, id: &str) -> io::Result<()> {
        let tmp_path = self.file_path(&format!("{}.tmp", id));
        let final_path = self.file_path(id);
        fs::rename(tmp_path, final_path)?;
        Ok(())
    }

    /// Deletes the .tmp file in case of an error (rollback).
    pub fn cleanup_tmp_file(&self, id: &str) {
        let tmp_path = self.file_path(&format!("{}.tmp", id));
        let _ = fs::remove_file(tmp_path);
    }

    /// Deletes ALL .tmp files (called at server startup).
    pub fn cleanup_all_tmp_files(&self) -> io::Result<usize> {
        let mut removed = 0;

        for entry in fs::read_dir(&self.root_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if name.ends_with(".tmp") {
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

        for entry in fs::read_dir(&self.root_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            if name.ends_with(".tmp") {
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

// ------------------ Public Methods ------------------

pub fn guess_mime(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .to_string()
}
