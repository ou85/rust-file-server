use bcrypt::{DEFAULT_COST, hash};
use std::io::{self, Write};

pub fn run(password: Option<String>) {
    let password = match password {
        Some(p) => p,
        None => {
            print!("\n==> Enter new `user` password: ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            input.trim().to_string()
        }
    };

    match hash(&password, DEFAULT_COST) {
        Ok(hashed) => println!(
            "\n=== Bcrypt hash:\n{}
         \n=== .env value:\nRFS_USER_PASSWORD_HASH='{}'\nRFS_ADMIN_PASSWORD_HASH='{}'\n",
            hashed, hashed, hashed
        ),
        Err(e) => eprintln!("Error: {}", e),
    }
}
