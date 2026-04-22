use serde::Serialize;
use std::fmt;

/// Parsec error codes by category.
///
/// Exit code mapping:
///   1 = general, 2 = auth, 3 = conflict, 4 = CI, 5 = state, 6 = policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub enum ErrorCode {
    // Auth (exit code 2)
    /// No token configured for the forge/tracker
    E001,
    // CI (exit code 4)
    /// CI checks are failing
    E002,
    // Conflict (exit code 3)
    /// File conflict across worktrees
    E003,
    // State (exit code 5)
    /// PR/MR is not mergeable
    E004,
    /// Workspace not found
    E005,
    /// Workspace already exists
    E006,
    /// No active workspaces
    E007,
    // General (exit code 1)
    /// Hook execution failed
    E008,
    // Policy (exit code 6)
    /// Policy violation (protected branch, disallowed target)
    E009,
    /// PR not found
    E010,
    /// Tracker/provider not configured or unsupported
    E011,
    /// Ship partial — pushed but PR creation failed
    E012,
    /// Cannot undo operation
    E013,
    /// General / uncategorized error
    E999,
}

impl ErrorCode {
    /// Process exit code for this error category.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::E001 => 2, // auth
            ErrorCode::E002 => 4, // CI
            ErrorCode::E003 => 3, // conflict
            ErrorCode::E004 | ErrorCode::E005 | ErrorCode::E006 | ErrorCode::E007 => 5, // state
            ErrorCode::E008 => 1, // general
            ErrorCode::E009 => 6, // policy
            ErrorCode::E010 => 5, // state
            ErrorCode::E011 => 2, // auth/config
            ErrorCode::E012 => 1, // general
            ErrorCode::E013 => 1, // general
            ErrorCode::E999 => 1, // general
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A structured parsec error carrying an error code.
#[derive(Debug)]
pub struct ParsecError {
    pub code: ErrorCode,
    pub message: String,
}

impl fmt::Display for ParsecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ParsecError {}

impl ParsecError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Structured JSON error envelope for `--json` mode.
#[derive(Serialize)]
pub struct JsonError {
    pub error: bool,
    pub code: ErrorCode,
    pub message: String,
}

/// Try to extract a [`ParsecError`] (and its code) from an `anyhow::Error` chain.
/// Falls back to `E999` for untyped errors.
pub fn extract_code(err: &anyhow::Error) -> (ErrorCode, &str) {
    if let Some(pe) = err.downcast_ref::<ParsecError>() {
        (pe.code, &pe.message)
    } else {
        (ErrorCode::E999, "")
    }
}

/// Convenience macro: `bail_code!(ErrorCode::E005, "workspace '{}' not found", ticket)`
#[macro_export]
macro_rules! bail_code {
    ($code:expr, $($arg:tt)*) => {
        return Err($crate::errors::ParsecError::new($code, format!($($arg)*)).into())
    };
}
