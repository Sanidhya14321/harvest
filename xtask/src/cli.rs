//! Shared standalone command parsing, including successful help exits.
use anyhow::Result;
use clap::Parser;

pub(crate) fn arguments<T: Parser>(name: &str, arguments: Vec<String>) -> Result<Option<T>> {
	match T::try_parse_from(std::iter::once(name.to_owned()).chain(arguments)) {
		Ok(parsed) => Ok(Some(parsed)),
		Err(help) if !help.use_stderr() => {
			help.print()?;
			Ok(None)
		},
		Err(error) => Err(error.into()),
	}
}
