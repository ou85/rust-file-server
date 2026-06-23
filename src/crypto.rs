use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8MB

#[derive(Clone)]
pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    pub fn new(secret: &str) -> Self {
        let hash = Sha256::digest(secret.as_bytes());
        let cipher = Aes256Gcm::new_from_slice(&hash).expect("invalid key");
        Self { cipher }
    }

    pub fn decrypt(&self, nonce_bytes: &[u8; 12], data: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher.decrypt(nonce, data) 
    }

    /// Encrypts data in chunks of CHUNK_SIZE.
    /// Format: [4 bytes chunk_size][12 bytes nonce][ciphertext+tag] * N
    pub fn encrypt_chunked(&self, data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let chunk_size = CHUNK_SIZE as u32;

        // Write the chunk size to the beginning of the file.
        output.extend_from_slice(&chunk_size.to_le_bytes());

        for chunk in data.chunks(CHUNK_SIZE) {
            let mut nonce_bytes = [0u8; 12];
            rand::rng().fill_bytes(&mut nonce_bytes);
            let nonce = Nonce::from_slice(&nonce_bytes);
            let encrypted = self.cipher.encrypt(nonce, chunk).unwrap();

            output.extend_from_slice(&nonce_bytes);
            output.extend_from_slice(&encrypted);
        }

        output
    }
}
