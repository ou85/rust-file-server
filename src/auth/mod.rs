use crate::config::Config;
use bcrypt::verify;

pub fn authenticate(username: &str, password: &str, config: &Config) -> Option<UserRole> {
    // println!("Input username: {}", username);
    // println!("Input password: {}", password);
    // println!("Stored user hash: {}", &config.user_password_hash);
    // println!("Stored admin hash: {}", &config.admin_password_hash);

    match username {
        u if u == config.user_name => {
            if verify(password, &config.user_password_hash).ok()? {
                Some(UserRole::User)
            } else {
                None
            }
        }
        a if a == config.admin_name => {
            if verify(password, &config.admin_password_hash).ok()? {
                Some(UserRole::Admin)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum UserRole {
    User,
    Admin,
}
