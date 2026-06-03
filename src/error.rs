use std::fmt;

#[derive(Debug)]
pub enum PkgdsError {
    Config(String),
    Io(std::io::Error),
    ParsePacmanConf(String),
}

impl fmt::Display for PkgdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkgdsError::Config(msg) => write!(f, "config error: {}", msg),
            PkgdsError::Io(err) => write!(f, "io error: {}", err),
            PkgdsError::ParsePacmanConf(msg) => write!(f, "parse pacman conf error: {}", msg),
        }
    }
}

impl std::error::Error for PkgdsError {}

impl From<std::io::Error> for PkgdsError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<regex::Error> for PkgdsError {
    fn from(value: regex::Error) -> Self {
        Self::Config(format!("regex parse error: {}", value))
    }
}

pub type Result<T> = std::result::Result<T, PkgdsError>;
