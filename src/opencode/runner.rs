//! Running OpenCode's own commands for verification and credential access.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MODELS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Check {
    pub ok: bool,
    pub output: String,
}

/// Run `opencode models` and confirm every model the installer wrote is
/// listed. The command reads the live configuration, so it proves the file
/// OpenCode actually loaded, not just the file on disk.
pub fn models_check(opencode: &Path, expected: &[String]) -> Check {
    let (success, stdout, stderr) = match run(opencode, &["models"], MODELS_TIMEOUT) {
        Ok(result) => result,
        Err(error) => {
            return Check {
                ok: false,
                output: error,
            };
        }
    };
    let output = strip_ansi(format!("{stdout}\n{stderr}").trim());
    let missing: Vec<&String> = expected
        .iter()
        .filter(|model| !output.contains(model.as_str()))
        .collect();
    if success && missing.is_empty() {
        return Check { ok: true, output };
    }
    let detail = if !success {
        format!("opencode models exited with an error.\n{output}")
    } else {
        format!(
            "opencode models did not list: {}\n{output}",
            missing
                .iter()
                .map(|model| model.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Check {
        ok: false,
        output: detail,
    }
}

/// Start a command, capture both streams on their own threads, and kill it if
/// it overruns. Returns `(success, stdout, stderr)` or a readable failure.
pub fn run(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> std::result::Result<(bool, String, String), String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not run {}: {error}", program.display()))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let read = |pipe: Option<std::process::ChildStdout>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let stderr_read = |pipe: Option<std::process::ChildStderr>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let stdout_thread = read(stdout_pipe);
    let stderr_thread = stderr_read(stderr_pipe);

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not wait for {}: {error}", program.display()));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();

    match status {
        Some(status) => Ok((status.success(), stdout, stderr)),
        None => Err(format!(
            "{} {} did not finish within {} seconds.",
            program.display(),
            args.join(" "),
            timeout.as_secs()
        )),
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
