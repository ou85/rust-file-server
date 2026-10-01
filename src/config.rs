use std::path::PathBuf;

#[derive(Debug)]
pub enum ConfigError {
    Arguments(String),
    Environment {
        name: &'static str,
        source: std::env::VarError,
    },
    ExecutablePath(std::io::Error),
    MissingExecutableDirectory,
    InvalidPort(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arguments(message) => f.write_str(message),
            Self::Environment { name, .. } => write!(
                f,
                "Set {name} to a valid UTF-8 value before starting the server"
            ),
            Self::ExecutablePath(error) => {
                write!(f, "Could not determine executable path: {error}")
            }
            Self::MissingExecutableDirectory => {
                f.write_str("Executable path has no parent directory")
            }
            Self::InvalidPort(value) => write!(
                f,
                "PORT must be an integer between 0 and 65535; received {value:?}"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Environment { source, .. } => Some(source),
            Self::ExecutablePath(error) => Some(error),
            _ => None,
        }
    }
}

fn required_env(name: &'static str) -> Result<String, ConfigError> {
    std::env::var(name).map_err(|source| ConfigError::Environment { name, source })
}

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
    pub fn from_args(args: &[String]) -> Result<Self, ConfigError> {
        let mut local_only = false;
        let mut data_dir = None;
        let mut index = 0;

        while index < args.len() {
            match args[index].as_str() {
                "--local" => local_only = true,
                "--data-dir" => {
                    index += 1;
                    let value = args.get(index).ok_or_else(|| {
                        ConfigError::Arguments("--data-dir requires a directory path".into())
                    })?;
                    data_dir = Some(PathBuf::from(value));
                }
                "--help" | "-h" => return Err(ConfigError::Arguments(usage())),
                option => {
                    return Err(ConfigError::Arguments(format!(
                        "Unknown option: {option}\n\n{}",
                        usage()
                    )));
                }
            }
            index += 1;
        }

        let data_dir = match data_dir {
            Some(path) => path,
            None => std::env::var_os("RFS_DATA_DIR")
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(default_data_dir)?,
        };

        Ok(Self {
            blobs_dir: data_dir.join("blobs"),
            tmp_dir: data_dir.join("tmp"),
            metadata_path: data_dir.join("metadata.redb"),
            data_dir,
            encryption_key: required_env("RFS_ENCRYPTION_KEY")?,
            user_name: "user".to_string(),
            user_password_hash: required_env("RFS_USER_PASSWORD_HASH")?,
            admin_name: "admin".to_string(),
            admin_password_hash: required_env("RFS_ADMIN_PASSWORD_HASH")?,
            bind_address: bind_address(local_only)?,
        })
    }
}

fn bind_address(local_only: bool) -> Result<String, ConfigError> {
    let port = match std::env::var("PORT") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => "3000".to_string(),
        Err(source) => {
            return Err(ConfigError::Environment {
                name: "PORT",
                source,
            });
        }
    };
    let port = parse_port(port)?;
    let host = if local_only { "127.0.0.1" } else { "0.0.0.0" };
    Ok(format!("{host}:{port}"))
}

fn parse_port(value: String) -> Result<u16, ConfigError> {
    value.parse().map_err(|_| ConfigError::InvalidPort(value))
}

fn default_data_dir() -> Result<PathBuf, ConfigError> {
    let executable = std::env::current_exe().map_err(ConfigError::ExecutablePath)?;
    let executable_dir = executable
        .parent()
        .ok_or(ConfigError::MissingExecutableDirectory)?;
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
    use super::{ConfigError, bind_address, parse_port};

    #[test]
    fn invalid_configuration_has_actionable_errors() {
        for value in ["bad", "65536", "-1"] {
            let error = parse_port(value.into()).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidPort(_)));
            assert!(error.to_string().contains("PORT must be an integer"));
        }
        let error = ConfigError::Environment {
            name: "RFS_ENCRYPTION_KEY",
            source: std::env::VarError::NotPresent,
        };
        assert!(error.to_string().contains("Set RFS_ENCRYPTION_KEY"));
    }

    #[test]
    fn local_flag_binds_loopback() {
        assert_eq!(bind_address(true).unwrap(), "127.0.0.1:3000");
    }

    #[test]
    fn network_default_binds_all_interfaces() {
        assert_eq!(bind_address(false).unwrap(), "0.0.0.0:3000");
    }
}
