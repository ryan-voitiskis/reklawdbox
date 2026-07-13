#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Subprocess(String),
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Analysis(String),
}
