use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    pub fn new(secret: &str) -> Self {
        let hash = Sha256::digest(secret.as_bytes());

        let cipher = Aes256Gcm::new_from_slice(&hash).expect("invalid key");

        Self { cipher }
    }

    pub fn encrypt(&self, data: &[u8]) -> ([u8; 12], Vec<u8>) {
        let mut nonce_bytes = [0u8; 12];

        rand::rng().fill_bytes(&mut nonce_bytes);

        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted = self.cipher.encrypt(nonce, data).unwrap();

        (nonce_bytes, encrypted)
    }

    pub fn decrypt(&self, nonce_bytes: &[u8; 12], data: &[u8]) -> Vec<u8> {
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher.decrypt(nonce, data).unwrap()
    }
}
