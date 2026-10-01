use bcrypt::{DEFAULT_COST, hash};
use std::io::{self, Write};

pub fn run(password: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let password = match password {
        Some(p) => p,
        None => {
            print!("\n==> Enter new `user` password: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    let hashed = hash(&password, DEFAULT_COST)?;
    println!(
        "\n=== Bcrypt hash:\n{}
         \n=== .env value:\nRFS_USER_PASSWORD_HASH='{}'\nRFS_ADMIN_PASSWORD_HASH='{}'\n",
        hashed, hashed, hashed
    );
    Ok(())
}
