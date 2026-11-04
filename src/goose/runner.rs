//! Running goose's own diagnostics.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CHECK_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
pub struct Check {
    pub ok: bool,
    pub output: String,
}

pub fn info_check(goose: &Path) -> Check {
    let mut child = match Command::new(goose)
        .args(["info", "--check"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Check {
                ok: false,
                output: format!("Could not run {}: {error}", goose.display()),
            };
        }
    };

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < CHECK_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Check {
                    ok: false,
                    output: "goose info --check did not finish within 45 seconds.".to_string(),
                };
            }
            Err(error) => {
                return Check {
                    ok: false,
                    output: format!("Could not wait for goose: {error}"),
                };
            }
        }
    };

    // try_wait has reaped the child; the pipes are still readable.
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(out) = child.stdout.take() {
        let mut out = out;
        let _ = std::io::Read::read_to_string(&mut out, &mut stdout);
    }
    if let Some(err) = child.stderr.take() {
        let mut err = err;
        let _ = std::io::Read::read_to_string(&mut err, &mut stderr);
    }
    let success = status.is_some_and(|status| status.success());

    Check {
        ok: success,
        output: strip_ansi(format!("{stdout}\n{stderr}").trim()),
    }
}

/// Remove ANSI escape sequences so diagnostics stay readable in a log.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(strip_ansi("\u{1b}[32mok\u{1b}[0m done"), "ok done");
    }
}
