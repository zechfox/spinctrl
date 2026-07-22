use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpinCtrlError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    
    #[error("Configuration validation error: {0:?}")]
    ConfigValidation(Vec<String>),
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("Service communication error: {0}")]
    ServiceComm(String),
    
    #[error("Hardware operation failed: {0}")]
    Hardware(String),
    
    #[error("Invalid value: {0}")]
    InvalidValue(String),
    
    #[error("File not found: {0}")]
    FileNotFound(String),
    
    #[error("Directory creation failed: {0}")]
    DirectoryCreation(String),
}

pub type Result<T> = std::result::Result<T, SpinCtrlError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_error_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let err: SpinCtrlError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("IO error"), "msg was: {msg}");
        assert!(msg.contains("missing file"), "msg was: {msg}");
    }

    #[test]
    fn test_json_error_from_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: SpinCtrlError = json_err.into();
        assert!(err.to_string().contains("JSON parsing error"));
    }

    #[test]
    fn test_config_validation_display_lists_errors() {
        let err = SpinCtrlError::ConfigValidation(vec!["err one".to_string(), "err two".to_string()]);
        let msg = err.to_string();
        assert!(msg.contains("Configuration validation error"));
        assert!(msg.contains("err one"));
        assert!(msg.contains("err two"));
    }

    #[test]
    fn test_all_string_variant_displays() {
        assert!(SpinCtrlError::PermissionDenied("denied".into()).to_string().contains("Permission denied"));
        assert!(SpinCtrlError::ServiceComm("comm fail".into()).to_string().contains("Service communication error"));
        assert!(SpinCtrlError::Hardware("hw fail".into()).to_string().contains("Hardware operation failed"));
        assert!(SpinCtrlError::InvalidValue("bad value".into()).to_string().contains("Invalid value"));
        assert!(SpinCtrlError::FileNotFound("/path/missing".into()).to_string().contains("File not found"));
        assert!(SpinCtrlError::DirectoryCreation("/dir/missing".into()).to_string().contains("Directory creation failed"));
    }

    #[test]
    fn test_result_alias_is_error_variant() {
        let res: Result<()> = Err(SpinCtrlError::InvalidValue("x".into()));
        assert!(res.is_err());
    }
}