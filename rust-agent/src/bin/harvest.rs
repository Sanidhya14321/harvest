//! Rust coding agent for Harvest.

use std::{
	io::{self, Write},
	process::ExitCode,
};

fn main() -> ExitCode {
	match harvest_omp::execute_cli() {
		Ok(code) => ExitCode::from(code),
		Err(error) => {
			report(&error);
			ExitCode::from(1)
		},
	}
}

fn report(error: &anyhow::Error) {
	let mut sink = io::stderr().lock();
	match writeln!(sink, "omp: {error:#}") {
		Ok(()) => {},
		Err(write_error) => drop(write_error),
	}
}
