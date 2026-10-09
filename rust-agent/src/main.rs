#![doc = "Rust coding agent for Harvest."]

mod agent;
mod chat;
mod cli;
mod config;
mod pathsafe;
mod provider;
mod session;
mod shell;
mod tools;

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::execute() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            report(&error);
            ExitCode::from(1)
        }
    }
}

fn report(error: &anyhow::Error) {
    let mut sink = io::stderr().lock();
    match writeln!(sink, "omp: {error:#}") {
        Ok(()) => {}
        Err(write_error) => drop(write_error),
    }
}
