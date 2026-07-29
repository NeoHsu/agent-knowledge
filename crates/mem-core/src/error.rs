use std::fmt;

use anyhow::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCode {
    Compatibility,
    Conflict,
    Integrity,
    NotFound,
    SafetyViolation,
    Usage,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compatibility => "compatibility",
            Self::Conflict => "conflict",
            Self::Integrity => "integrity",
            Self::NotFound => "not_found",
            Self::SafetyViolation => "safety_violation",
            Self::Usage => "usage",
        }
    }

    pub fn exit_code(self) -> u8 {
        match self {
            Self::Compatibility | Self::SafetyViolation | Self::Usage => 2,
            Self::NotFound => 4,
            Self::Conflict | Self::Integrity => 1,
        }
    }
}

#[derive(Debug)]
pub struct MnemarkError {
    code: ErrorCode,
    message: String,
}

impl MnemarkError {
    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for MnemarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MnemarkError {}

fn classified(code: ErrorCode, message: impl Into<String>) -> Error {
    MnemarkError {
        code,
        message: message.into(),
    }
    .into()
}

pub fn compatibility(message: impl Into<String>) -> Error {
    classified(ErrorCode::Compatibility, message)
}

pub fn conflict(message: impl Into<String>) -> Error {
    classified(ErrorCode::Conflict, message)
}

pub fn integrity(message: impl Into<String>) -> Error {
    classified(ErrorCode::Integrity, message)
}

pub fn not_found(message: impl Into<String>) -> Error {
    classified(ErrorCode::NotFound, message)
}

pub fn safety_violation(message: impl Into<String>) -> Error {
    classified(ErrorCode::SafetyViolation, message)
}

pub fn usage(message: impl Into<String>) -> Error {
    classified(ErrorCode::Usage, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifications_keep_stable_codes_and_exit_statuses() {
        let cases = [
            (compatibility("compatibility"), "compatibility", 2),
            (conflict("conflict"), "conflict", 1),
            (integrity("integrity"), "integrity", 1),
            (not_found("not found"), "not_found", 4),
            (safety_violation("unsafe"), "safety_violation", 2),
            (usage("invalid input"), "usage", 2),
        ];

        for (error, expected_code, expected_exit) in cases {
            let actual = error
                .downcast_ref::<MnemarkError>()
                .map(|classified| (classified.code().as_str(), classified.code().exit_code()));
            assert_eq!(actual, Some((expected_code, expected_exit)));
        }
    }
}
