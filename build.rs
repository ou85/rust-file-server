use std::{env, fs};

// Default key used when no build-time override is supplied. Keep it stable so
// binaries built without an explicit key can still open existing data.
const DEFAULT_ENCRYPTION_KEY: &str = "C9GYOyZV1HXXEbGMnFrUAihGj0dM3xpNvGWd7MDjnds=";

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
    let key = key
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_ENCRYPTION_KEY.to_owned());
    println!("cargo:rustc-env=RFS_EMBEDDED_ENCRYPTION_KEY={key}");
    println!("cargo:warning=Encryption key is embedded in this binary");
}
