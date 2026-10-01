use std::path::PathBuf;

pub struct Config {
    pub data_dir: PathBuf,
    pub blobs_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub metadata_path: PathBuf,
    pub encryption_key: String,

    pub user_name: String,
    pub admin_name: String,
    pub user_password_hash: String,
    pub admin_password_hash: String,

    pub bind_address: String,
}

impl Config {
    pub fn from_args(args: &[String]) -> Result<Self, String> {
        let mut local_only = false;
        let mut data_dir = None;
        let mut index = 0;

        while index < args.len() {
            match args[index].as_str() {
                "--local" => local_only = true,
                "--data-dir" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or("--data-dir requires a directory path")?;
                    data_dir = Some(PathBuf::from(value));
                }
                "--help" | "-h" => return Err(usage()),
                option => return Err(format!("Unknown option: {option}\n\n{}", usage())),
            }
            index += 1;
        }

        let data_dir = match data_dir {
            Some(path) => path,
            None => std::env::var_os("RFS_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(default_data_dir()?),
        };

        Ok(Self {
            blobs_dir: data_dir.join("blobs"),
            tmp_dir: data_dir.join("tmp"),
            metadata_path: data_dir.join("metadata.redb"),
            data_dir,
            encryption_key: std::env::var("RFS_ENCRYPTION_KEY")
                .expect("RFS_ENCRYPTION_KEY is not set"),
            user_name: "user".to_string(),
            user_password_hash: std::env::var("RFS_USER_PASSWORD_HASH")
                .expect("RFS_USER_PASSWORD_HASH is not set"),
            admin_name: "admin".to_string(),
            admin_password_hash: std::env::var("RFS_ADMIN_PASSWORD_HASH")
                .expect("RFS_ADMIN_PASSWORD_HASH is not set"),
            bind_address: bind_address(local_only),
        })
    }
}

fn bind_address(local_only: bool) -> String {
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let host = if local_only { "127.0.0.1" } else { "0.0.0.0" };
    format!("{host}:{port}")
}

fn default_data_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not determine executable path: {error}"))?;
    let executable_dir = executable
        .parent()
        .ok_or("Executable path has no parent directory")?;
    Ok(executable_dir.join("data"))
}

pub fn usage() -> String {
    format!(
        "Usage: {} [--local] [--data-dir <PATH>]\n\n\
         By default the server listens on 0.0.0.0 and stores data next to the executable.\n\
         --local            listen only on 127.0.0.1\n\
         --data-dir <PATH>  data directory (also configurable with RFS_DATA_DIR)",
        std::env::args().next().unwrap_or_else(|| "rfs".to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::bind_address;

    #[test]
    fn local_flag_binds_loopback() {
        assert_eq!(bind_address(true), "127.0.0.1:3000");
    }

    #[test]
    fn network_default_binds_all_interfaces() {
        assert_eq!(bind_address(false), "0.0.0.0:3000");
    }
}
