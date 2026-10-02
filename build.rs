use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let key = env::var("RFS_ENCRYPTION_KEY").ok().or_else(|| {
        fs::read_to_string(".env").ok().and_then(|contents| {
            contents.lines().find_map(|line| {
                let value = line.trim().strip_prefix("RFS_ENCRYPTION_KEY=")?;
                Some(value.trim_matches([' ', '\'', '"']).to_owned())
            })
        })
    });
    let Some(key) = key.filter(|value| !value.is_empty()) else {
        panic!("RFS_ENCRYPTION_KEY must be supplied while building (environment or .env)");
    };
    println!("cargo:rustc-env=RFS_EMBEDDED_ENCRYPTION_KEY={key}");
    println!("cargo:warning=Encryption key is embedded in this binary");
}
