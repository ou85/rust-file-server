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

    // pub fn encrypt(&self, data: &[u8]) -> ([u8; 12], Vec<u8>) {
    //     let mut nonce_bytes = [0u8; 12];
    //     rand::rng().fill_bytes(&mut nonce_bytes);
    //     let nonce = Nonce::from_slice(&nonce_bytes);
    //     let encrypted = self.cipher.encrypt(nonce, data).unwrap();
    //     (nonce_bytes, encrypted)
    // }

    // pub fn decrypt(&self, nonce_bytes: &[u8; 12], data: &[u8]) -> Vec<u8> {
    //     let nonce = Nonce::from_slice(nonce_bytes);
    //     self.cipher.decrypt(nonce, data).unwrap()
    // }

    pub fn decrypt(&self, nonce_bytes: &[u8; 12], data: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher.decrypt(nonce, data) // убираем .unwrap()
    }

    /// Шифрует data чанками по CHUNK_SIZE.
    /// Формат: [4 bytes chunk_size][12 bytes nonce][ciphertext+tag] * N
    pub fn encrypt_chunked(&self, data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let chunk_size = CHUNK_SIZE as u32;

        // Записываем размер чанка в начало файла
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

    // pub fn decrypt_chunk(&self, data: &[u8]) -> Option<(Vec<u8>, usize)> {
    //     if data.len() < 12 {
    //         return None;
    //     }
    //     // nonce — первые 12 байт
    //     let nonce_bytes: [u8; 12] = data[..12].try_into().ok()?;
    //     // за nonce идут ciphertext + 16-байтный GCM-тег
    //     // размер ciphertext неизвестен — берём CHUNK_SIZE + 16 (тег) или остаток
    //     let ct_end = (12 + CHUNK_SIZE + 16).min(data.len());
    //     let ciphertext = &data[12..ct_end];

    //     let nonce = Nonce::from_slice(&nonce_bytes);
    //     let plaintext = self.cipher.decrypt(nonce, ciphertext).ok()?;

    //     Some((plaintext, ct_end))
    // }
}
