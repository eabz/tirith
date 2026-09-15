//! The `tirith` command-line entry point. Argument parsing lives in
//! `cli.rs`; everything else is in the library.

#![forbid(unsafe_code)]

mod cli;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match cli::run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
