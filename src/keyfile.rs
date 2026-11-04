//! Parsing of `--key-file`: a plain env file with the same names as the
//! environment variables.

use std::fs;
use std::io::{self, Read};

use crate::error::{Fail, Result, code};
use crate::fsutil;

#[derive(Debug, PartialEq, Eq)]
pub struct KeyFileEntry {
    pub name: String,
    pub value: String,
}

/// Parse env-file content. Errors carry a line number but never the line text,
/// so a key can not leak into logs through a parse error.
pub fn parse(text: &str) -> Result<Vec<KeyFileEntry>> {
    let mut entries: Vec<KeyFileEntry> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(Fail::new(
                code::USAGE,
                format!("Key file line {} is not NAME=value.", index + 1),
            )
            .hint("Use RHAI_MAAS_API_KEY=... and ENMAAS_API_KEY=... on their own lines."));
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(Fail::new(
                code::USAGE,
                format!("Key file line {} has an empty name.", index + 1),
            ));
        }
        if entries.iter().any(|e| e.name == name) {
            return Err(Fail::new(
                code::USAGE,
                format!("Key file line {} repeats {name}.", index + 1),
            )
            .hint("Remove the duplicate so the intended key is unambiguous."));
        }
        let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
        entries.push(KeyFileEntry { name, value });
    }
    Ok(entries)
}

/// Read and parse a key file. `-` reads stdin.
pub fn read(path: &str) -> Result<Vec<KeyFileEntry>> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map_err(|error| {
            Fail::new(code::KEY, format!("Could not read keys from stdin: {error}"))
        })?;
        return parse(&buf);
    }

    let path_buf = std::path::PathBuf::from(path);
    let meta = fs::metadata(&path_buf).map_err(|error| {
        Fail::new(
            code::KEY,
            format!("Could not read key file {path}: {error}"),
        )
    })?;
    if !meta.is_file() {
        return Err(Fail::new(
            code::KEY,
            format!("Key file {path} is not a regular file."),
        ));
    }
    if !fsutil::is_private_file(&path_buf).unwrap_or(false) {
        return Err(Fail::new(
            code::KEY,
            format!("Key file {path} is readable by other users."),
        )
        .hint(format!("Run: chmod 600 {path}")));
    }

    let text = fs::read_to_string(&path_buf).map_err(|error| {
        Fail::new(
            code::KEY,
            format!("Could not read key file {path}: {error}"),
        )
    })?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_blank_lines_and_wrapped_values() {
        let entries = parse(
            "# comment\n\nRHAI_MAAS_API_KEY=abc 123\nENMAAS_API_KEY=def\r\nUNRELATED=1\n",
        )
        .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "RHAI_MAAS_API_KEY");
        assert_eq!(entries[0].value, "abc123");
        assert_eq!(entries[1].value, "def");
    }

    #[test]
    fn rejects_duplicates() {
        let err = parse("A=1\nA=2\n").unwrap_err();
        assert_eq!(err.code, code::USAGE);
        assert!(err.message.contains("line 2"));
        assert!(err.message.contains("repeats A"));
    }

    #[test]
    fn rejects_malformed_without_echoing_content() {
        let err = parse("WHOOPS\n").unwrap_err();
        assert!(err.message.contains("line 1"));
        assert!(!err.message.contains("WHOOPS"));
    }
}
