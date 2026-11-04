mod apply;
mod catalog;
mod cli;
mod error;
mod fsutil;
mod goose;
mod keyfile;
mod netcheck;
mod output;
mod platform;
mod settings;
mod tui;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let args = cli::Args::parse();
    let format = args.format;
    match cli::run(args) {
        Ok(code) => ExitCode::from(code),
        Err(fail) => {
            if matches!(format, cli::Format::Json) {
                let value = serde_json::json!({
                    "ok": false,
                    "error": fail.message,
                    "errorCode": fail.code,
                    "hints": fail.hints,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            }
            output::print_fail(&fail);
            ExitCode::from(fail.code)
        }
    }
}
