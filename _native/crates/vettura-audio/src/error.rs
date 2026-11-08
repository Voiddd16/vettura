use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, AudioError>;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sfx directory '{}' does not exist", .0.display())]
    MissingFolder(PathBuf),

    #[error("sfx directory '{}' has no car soundbank", .0.display())]
    MissingBank(PathBuf),

    #[error("sfx directory '{}' has multiple candidate banks: {}", .0.display(), .1.join(", "))]
    AmbiguousBank(PathBuf, Vec<String>),

    #[error("GUIDs.txt line {line} is invalid: {text:?}")]
    Guids { line: usize, text: String },

    #[error("invalid FMOD GUID: {0}")]
    Guid(String),

    #[error("unknown sound event: {0}")]
    UnknownEvent(String),

    #[error("FMOD runtime error: {0}")]
    Fmod(String),

    /// Built without the `fmod` feature, so no FMOD runtime is linked.
    #[error("FMOD runtime not linked; rebuild with `--features fmod`")]
    NotLinked,
}

impl AudioError {
    pub(crate) fn invalid_guids_line(line: usize, text: &str) -> Self {
        Self::Guids {
            line,
            text: text.to_owned(),
        }
    }
}
