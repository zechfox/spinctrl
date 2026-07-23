use thiserror::Error;

#[derive(Error, Debug)]
pub enum TuiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Shared library error: {0}")]
    Shared(#[from] shared::SpinCtrlError),
}

pub type Result<T> = std::result::Result<T, TuiError>;

#[cfg(test)]
mod tests {
    use super::*;
    use shared::SpinCtrlError;
    use std::io;

    #[test]
    fn test_io_error_from_conversion() {
        let io_err = io::Error::other("io boom");
        let err: TuiError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("IO error"), "msg was: {msg}");
        assert!(msg.contains("io boom"), "msg was: {msg}");
    }

    #[test]
    fn test_shared_error_from_conversion() {
        let shared_err = SpinCtrlError::InvalidValue("bad value".into());
        let err: TuiError = shared_err.into();
        let msg = err.to_string();
        assert!(msg.contains("Shared library error"), "msg was: {msg}");
        assert!(msg.contains("Invalid value"), "msg was: {msg}");
        assert!(msg.contains("bad value"), "msg was: {msg}");
    }

    #[test]
    fn test_result_alias_is_error_variant() {
        let res: Result<()> = Err(TuiError::Io(io::Error::other("oops")));
        assert!(res.is_err());
    }
}