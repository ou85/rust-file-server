use base64::{Engine, engine::general_purpose};
// use rand::Rng;
use rand::RngExt;

pub fn run(size: usize) {
    let mut rng = rand::rng();
    let key: Vec<u8> = (0..size).map(|_| rng.random()).collect();

    let encoded = general_purpose::STANDARD.encode(&key);
    println!("\n=== Encryption Key (Base64):\n{}", encoded);
    println!("\n=== .env value:");
    println!("RFS_ENCRYPTION_KEY={}", encoded);
    println!("\n=== Encryption Key (Hex):");
    println!("{}", hex::encode(&key));
}
