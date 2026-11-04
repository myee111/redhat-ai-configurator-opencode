//! Stable failure type and process exit codes.
//!
//! The codes are documented in the man page and relied on by automation, so
//! they must not be renumbered.

use std::fmt;

pub mod code {
    pub const INTERNAL: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const GOOSE: u8 = 3;
    pub const KEY: u8 = 4;
    pub const NETWORK: u8 = 5;
    pub const WRITE: u8 = 6;
    pub const VERIFY: u8 = 7;
}

#[derive(Debug)]
pub struct Fail {
    pub code: u8,
    pub message: String,
    pub hints: Vec<String>,
}

impl Fail {
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hints: Vec::new(),
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Fail {}

pub type Result<T> = std::result::Result<T, Fail>;

impl From<std::io::Error> for Fail {
    fn from(value: std::io::Error) -> Self {
        Fail::new(code::WRITE, value.to_string())
    }
}
