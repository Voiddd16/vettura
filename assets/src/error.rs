use thiserror::Error;

pub type Result<T> = std::result::Result<T, AssetError>;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ACD error: {0}")]
    Acd(#[from] vettura_acdata::Error),

    #[error("KN5 error: {0}")]
    Kn5(#[from] vettura_kn5::Error),

    #[error("car folder '{}' has neither data.acd nor data/", .0.display())]
    MissingData(std::path::PathBuf),

    #[error("data file '{0}' is not present")]
    MissingFile(String),

    #[error("data file '{0}' is not valid UTF-8 text")]
    NotText(String),

    #[error("invalid car data: {0}")]
    InvalidData(String),
}
