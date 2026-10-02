use bcrypt::{DEFAULT_COST, hash};

pub fn run(password: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let password = match password {
        Some(p) => p,
        None => super::password::prompt("\n==> Enter new `user` password: ")?,
    };

    let hashed = hash(&password, DEFAULT_COST)?;
    println!(
        "\n=== Bcrypt hash:\n{}
         \n=== .env value:\nRFS_USER_PASSWORD_HASH='{}'\nRFS_ADMIN_PASSWORD_HASH='{}'\n",
        hashed, hashed, hashed
    );
    Ok(())
}
