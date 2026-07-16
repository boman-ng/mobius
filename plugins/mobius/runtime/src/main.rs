#![forbid(unsafe_code)]

pub mod application;
pub mod domain;
mod error;
mod infrastructure;
mod presentation;
mod transport;

use std::process::ExitCode;

fn main() -> ExitCode {
    match transport::run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.to_json());
            ExitCode::from(error.exit_code())
        }
    }
}
