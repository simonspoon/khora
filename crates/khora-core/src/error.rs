use thiserror::Error;

/// Core error type for all Khora operations.
#[derive(Debug, Error)]
pub enum KhoraError {
    #[error("element not found: {0}")]
    ElementNotFound(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session expired or Chrome process died: {0}")]
    SessionDead(String),

    #[error("navigation failed: {0}")]
    NavigationFailed(String),

    #[error("timed out after {0}ms")]
    Timeout(u64),

    #[error("screenshot failed: {0}")]
    ScreenshotFailed(String),

    #[error("JavaScript error: {0}")]
    JavaScriptError(String),

    #[error("Chrome not found — install Chrome or set CHROME_PATH")]
    ChromeNotFound,

    #[error("Chrome launch failed: {0}")]
    LaunchFailed(String),

    #[error("CDP error: {0}")]
    Cdp(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl KhoraError {
    /// Map error variants to CLI exit codes.
    pub fn exit_code(&self) -> i32 {
        match self {
            KhoraError::ChromeNotFound => 2,
            KhoraError::Timeout(_) => 3,
            KhoraError::SessionNotFound(_) | KhoraError::SessionDead(_) => 4,
            _ => 1,
        }
    }
}

pub type KhoraResult<T> = Result<T, KhoraError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_code_chrome_not_found() {
        assert_eq!(KhoraError::ChromeNotFound.exit_code(), 2);
    }

    #[test]
    fn test_exit_code_timeout() {
        assert_eq!(KhoraError::Timeout(5000).exit_code(), 3);
    }

    #[test]
    fn test_exit_code_session_not_found() {
        assert_eq!(KhoraError::SessionNotFound("abc".into()).exit_code(), 4);
    }

    #[test]
    fn test_exit_code_session_dead() {
        assert_eq!(KhoraError::SessionDead("abc".into()).exit_code(), 4);
    }

    #[test]
    fn test_exit_code_defaults_to_1() {
        assert_eq!(KhoraError::ElementNotFound("x".into()).exit_code(), 1);
        assert_eq!(KhoraError::NavigationFailed("x".into()).exit_code(), 1);
        assert_eq!(KhoraError::LaunchFailed("x".into()).exit_code(), 1);
        assert_eq!(KhoraError::ScreenshotFailed("x".into()).exit_code(), 1);
        assert_eq!(KhoraError::JavaScriptError("x".into()).exit_code(), 1);
        assert_eq!(KhoraError::Cdp("x".into()).exit_code(), 1);
    }

    #[test]
    fn test_display_messages() {
        assert_eq!(
            KhoraError::ElementNotFound("div.btn".into()).to_string(),
            "element not found: div.btn"
        );
        assert_eq!(
            KhoraError::Timeout(3000).to_string(),
            "timed out after 3000ms"
        );
        assert_eq!(
            KhoraError::ChromeNotFound.to_string(),
            "Chrome not found — install Chrome or set CHROME_PATH"
        );
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let khora_err: KhoraError = io_err.into();
        assert_eq!(khora_err.exit_code(), 1);
        assert!(khora_err.to_string().contains("file missing"));
    }

    #[test]
    fn test_json_error_conversion() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let khora_err: KhoraError = json_err.into();
        assert_eq!(khora_err.exit_code(), 1);
    }
}
