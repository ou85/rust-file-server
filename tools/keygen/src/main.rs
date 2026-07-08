use base64::Engine;
use rand::Rng;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let key_size = if args.len() > 1 {
        args[1].parse::<usize>().unwrap_or(32)
    } else {
        32
    };

    let mut rng = rand::thread_rng();
    let key: Vec<u8> = (0..key_size).map(|_| rng.gen::<u8>()).collect();

    let engine = base64::engine::general_purpose::STANDARD;
    let encoded = engine.encode(&key);

    println!("RFS_ENCRYPTION_KEY={}", encoded);
    println!("Hex: {}", hex::encode(&key));
}
