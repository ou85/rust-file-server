use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::convert::TryFrom;
use std::io;

pub const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8MB
const MAGIC: &[u8; 4] = b"RFS1";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 4 + 16;

#[derive(Clone)]
pub struct Crypto {
    master_key: [u8; 32],
    legacy_cipher: Aes256Gcm,
}

impl Crypto {
    pub fn new(secret: &str) -> Result<Self, String> {
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, secret)
            .or_else(|_| hex::decode(secret))
            .map_err(|_| "RFS_ENCRYPTION_KEY must be 32 bytes encoded as Base64 or hex")?;
        let master_key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| "RFS_ENCRYPTION_KEY must decode to exactly 32 bytes")?;
        let legacy_hash = Sha256::digest(secret.as_bytes());
        let legacy_cipher = Aes256Gcm::new_from_slice(&legacy_hash).expect("valid SHA-256 key");
        Ok(Self {
            master_key,
            legacy_cipher,
        })
    }

    pub fn encrypt_chunked(&self, file_id: &str, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut encryptor = self.start_encryption(file_id)?;
        let mut output = encryptor.header().to_vec();
        for chunk in data.chunks(CHUNK_SIZE) {
            output.extend_from_slice(&encryptor.encrypt_chunk(chunk)?);
        }
        Ok(output)
    }

    pub fn start_encryption(&self, file_id: &str) -> io::Result<Encryptor> {
        let mut salt = [0u8; 16];
        rand::rng().fill_bytes(&mut salt);
        let cipher = self.file_cipher(file_id, &salt);
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&(CHUNK_SIZE as u32).to_le_bytes());
        header.extend_from_slice(&salt);
        Ok(Encryptor { cipher, header })
    }

    pub fn parse_cipher(
        &self,
        file_id: &str,
        data: &[u8],
    ) -> io::Result<(usize, usize, Aes256Gcm)> {
        if data.starts_with(MAGIC) {
            if data.len() < HEADER_LEN || data[4] != VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported encrypted file format",
                ));
            }
            let chunk_size = u32::from_le_bytes(data[5..9].try_into().unwrap()) as usize;
            if chunk_size == 0 || chunk_size > CHUNK_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid chunk size",
                ));
            }
            let salt: [u8; 16] = data[9..HEADER_LEN].try_into().unwrap();
            Ok((HEADER_LEN, chunk_size, self.file_cipher(file_id, &salt)))
        } else if data.len() >= 4 {
            Ok((
                4,
                u32::from_le_bytes(data[..4].try_into().unwrap()) as usize,
                self.legacy_cipher.clone(),
            ))
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"))
        }
    }

    fn file_cipher(&self, file_id: &str, salt: &[u8; 16]) -> Aes256Gcm {
        let mut hash = Sha256::new();
        hash.update(self.master_key);
        hash.update(salt);
        hash.update(file_id.as_bytes());
        hash.update(b"rfs-file-key-v1");
        Aes256Gcm::new_from_slice(&hash.finalize()).expect("SHA-256 key")
    }
}

pub struct Encryptor {
    cipher: Aes256Gcm,
    header: Vec<u8>,
}
impl Encryptor {
    pub fn header(&self) -> &[u8] {
        &self.header
    }
    pub fn encrypt_chunk(&mut self, data: &[u8]) -> io::Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::try_from(nonce_bytes.as_slice()).unwrap();
        let encrypted = self
            .cipher
            .encrypt(&nonce, data)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut frame = nonce_bytes.to_vec();
        frame.extend_from_slice(&encrypted);
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn v1_file_round_trip_uses_its_own_key() {
        let crypto = Crypto::new(KEY).unwrap();
        let encrypted = crypto.encrypt_chunked("file-a", b"secret").unwrap();
        assert!(encrypted.starts_with(MAGIC));
        let (offset, _, cipher) = crypto.parse_cipher("file-a", &encrypted).unwrap();
        let nonce: [u8; 12] = encrypted[offset..offset + 12].try_into().unwrap();
        assert_eq!(
            cipher
                .decrypt(
                    &Nonce::try_from(nonce.as_slice()).unwrap(),
                    &encrypted[offset + 12..]
                )
                .unwrap(),
            b"secret"
        );
        let (_, _, wrong_cipher) = crypto.parse_cipher("file-b", &encrypted).unwrap();
        assert!(
            wrong_cipher
                .decrypt(
                    &Nonce::try_from(nonce.as_slice()).unwrap(),
                    &encrypted[offset + 12..]
                )
                .is_err()
        );
    }
}
