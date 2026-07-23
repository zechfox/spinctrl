use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Shared library error: {0}")]
    Shared(#[from] shared::SpinCtrlError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Hardware operation failed: {0}")]
    Hardware(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;