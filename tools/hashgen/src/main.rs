use bcrypt::{hash, DEFAULT_COST};
use std::io::{self, Write};

fn main() {
    print!("Input your password: ");
    io::stdout().flush().unwrap();

    let mut password = String::new();
    io::stdin().read_line(&mut password).unwrap();
    let password = password.trim();

    match hash(password, DEFAULT_COST) {
        Ok(hashed) => println!("Bcrypt hash: {}\nADMIN_PASSWORD_HASH='{}'", hashed, hashed),
        Err(e) => eprintln!("Error: {}", e),
    }
}
