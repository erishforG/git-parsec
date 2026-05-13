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

/// A structured parsec error carrying an error code, plus optional cause and
/// help text.
///
/// Issue #303 — error messages follow a 3-line standard so users can
/// distinguish *what failed*, *why*, and *what to do next*:
///
/// ```text
/// error: workspace 'CL-2283' not found [E005]
/// caused by: directory missing or .git/parsec/state.json out of sync
/// help: run `parsec doctor`, or `parsec clean --orphans` to drop stale state
/// ```
///
/// `caused_by` and `help` are optional. Existing call sites that only set
/// `message` keep rendering as a single line — the format is additive.
#[derive(Debug, Clone)]
pub struct ParsecError {
    pub code: ErrorCode,
    pub message: String,
    pub caused_by: Option<String>,
    pub help: Option<String>,
}

impl fmt::Display for ParsecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Line 1: error summary + code (always present)
        write!(f, "error: {} [{}]", self.message, self.code)?;
        // Line 2: optional `caused by`
        if let Some(ref cb) = self.caused_by {
            write!(f, "\ncaused by: {}", cb)?;
        }
        // Line 3: optional `help`
        if let Some(ref h) = self.help {
            write!(f, "\nhelp: {}", h)?;
        }
        Ok(())
    }
}

impl std::error::Error for ParsecError {}

impl ParsecError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            caused_by: None,
            help: None,
        }
    }

    /// Attach a `caused by` line — the upstream cause in plain language.
    pub fn with_caused_by(mut self, cause: impl Into<String>) -> Self {
        self.caused_by = Some(cause.into());
        self
    }

    /// Attach a `help` line — the next action the user should take.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Structured JSON error envelope for `--json` mode.
///
/// `caused_by` / `help` use `skip_serializing_if` so unset values don't appear
/// in the JSON output (older consumers continue to see the same shape they
/// always did — the additions are strictly opt-in per call site).
#[derive(Serialize)]
pub struct JsonError {
    pub error: bool,
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// Try to extract a [`ParsecError`] (and its code) from an `anyhow::Error`
/// chain. Falls back to `E999` for untyped errors.
///
/// Kept for backward compat with existing callers (returns just the code +
/// message). New code should prefer [`extract_full`].
pub fn extract_code(err: &anyhow::Error) -> (ErrorCode, &str) {
    if let Some(pe) = err.downcast_ref::<ParsecError>() {
        (pe.code, &pe.message)
    } else {
        (ErrorCode::E999, "")
    }
}

/// Like [`extract_code`] but also returns optional `caused_by` / `help`.
///
/// Returns `None` when the error is not a [`ParsecError`] — callers can fall
/// back to the raw `anyhow` chain in that case (typically `format!("{err:#}")`).
pub fn extract_full(err: &anyhow::Error) -> Option<&ParsecError> {
    err.downcast_ref::<ParsecError>()
}

/// Convenience macro: `bail_code!(ErrorCode::E005, "workspace '{}' not found", ticket)`
///
/// For `caused_by` / `help`, build the `ParsecError` directly:
///
/// ```ignore
/// return Err(ParsecError::new(ErrorCode::E005, format!("workspace '{}' not found", ticket))
///     .with_caused_by("directory missing")
///     .with_help("run `parsec doctor`")
///     .into());
/// ```
#[macro_export]
macro_rules! bail_code {
    ($code:expr, $($arg:tt)*) => {
        return Err($crate::errors::ParsecError::new($code, format!($($arg)*)).into())
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_single_line_when_no_cause_or_help() {
        let e = ParsecError::new(ErrorCode::E005, "workspace 'X' not found");
        assert_eq!(e.to_string(), "error: workspace 'X' not found [E005]");
    }

    #[test]
    fn display_two_lines_with_caused_by() {
        let e = ParsecError::new(ErrorCode::E005, "workspace 'X' not found")
            .with_caused_by("directory missing");
        assert_eq!(
            e.to_string(),
            "error: workspace 'X' not found [E005]\ncaused by: directory missing"
        );
    }

    #[test]
    fn display_three_lines_with_caused_by_and_help() {
        let e = ParsecError::new(ErrorCode::E005, "workspace 'X' not found")
            .with_caused_by("directory missing or state.json out of sync")
            .with_help("run `parsec doctor`, or `parsec clean --orphans`");
        let expected = "error: workspace 'X' not found [E005]\n\
                        caused by: directory missing or state.json out of sync\n\
                        help: run `parsec doctor`, or `parsec clean --orphans`";
        assert_eq!(e.to_string(), expected);
    }

    #[test]
    fn display_skips_caused_by_when_only_help_is_set() {
        // help-only is allowed too — useful for "you need to do X" hints
        // without a clear underlying cause.
        let e = ParsecError::new(ErrorCode::E001, "no token configured")
            .with_help("set PARSEC_GITHUB_TOKEN or run `gh auth login`");
        assert_eq!(
            e.to_string(),
            "error: no token configured [E001]\nhelp: set PARSEC_GITHUB_TOKEN or run `gh auth login`"
        );
    }

    #[test]
    fn extract_full_returns_typed_error() {
        let pe = ParsecError::new(ErrorCode::E005, "msg")
            .with_caused_by("cb")
            .with_help("h");
        let err: anyhow::Error = pe.into();
        let extracted = extract_full(&err).expect("typed error");
        assert_eq!(extracted.code, ErrorCode::E005);
        assert_eq!(extracted.caused_by.as_deref(), Some("cb"));
        assert_eq!(extracted.help.as_deref(), Some("h"));
    }

    #[test]
    fn extract_full_returns_none_for_untyped_error() {
        let err = anyhow::anyhow!("plain error");
        assert!(extract_full(&err).is_none());
    }

    #[test]
    fn extract_code_backward_compat_for_typed() {
        let pe = ParsecError::new(ErrorCode::E007, "no active workspaces");
        let err: anyhow::Error = pe.into();
        let (code, msg) = extract_code(&err);
        assert_eq!(code, ErrorCode::E007);
        assert_eq!(msg, "no active workspaces");
    }

    #[test]
    fn extract_code_backward_compat_for_untyped() {
        let err = anyhow::anyhow!("plain");
        let (code, msg) = extract_code(&err);
        assert_eq!(code, ErrorCode::E999);
        assert_eq!(msg, "");
    }

    #[test]
    fn json_error_omits_unset_fields() {
        let je = JsonError {
            error: true,
            code: ErrorCode::E005,
            message: "msg".to_string(),
            caused_by: None,
            help: None,
        };
        let s = serde_json::to_string(&je).unwrap();
        // Backward-compat: existing JSON consumers see the same 3 keys.
        assert!(!s.contains("caused_by"));
        assert!(!s.contains("\"help\""));
        assert!(s.contains("\"code\":\"E005\""));
        assert!(s.contains("\"message\":\"msg\""));
    }

    #[test]
    fn json_error_includes_set_fields() {
        let je = JsonError {
            error: true,
            code: ErrorCode::E005,
            message: "msg".to_string(),
            caused_by: Some("cb".to_string()),
            help: Some("h".to_string()),
        };
        let s = serde_json::to_string(&je).unwrap();
        assert!(s.contains("\"caused_by\":\"cb\""));
        assert!(s.contains("\"help\":\"h\""));
    }

    #[test]
    fn bail_code_macro_still_works() {
        fn doit() -> anyhow::Result<()> {
            bail_code!(ErrorCode::E005, "ticket {} missing", "X");
        }
        let err = doit().unwrap_err();
        let (code, msg) = extract_code(&err);
        assert_eq!(code, ErrorCode::E005);
        assert_eq!(msg, "ticket X missing");
    }
}
