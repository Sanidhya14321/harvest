use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;

use crate::chat::ToolCall;
use crate::pathsafe::workspace_file;

const OUTPUT_LIMIT: usize = 20_000;
const MATCH_LIMIT: usize = 50;

pub(crate) fn invoke(root: &Path, call: &ToolCall) -> Result<String> {
    let name = call.function.name.as_str();
    let arguments = call.function.arguments.as_str();
    let output = match name {
        "read" => read_file(root, &parse::<ReadArgs>(arguments)?)?,
        "write" => write_file(root, &parse::<WriteArgs>(arguments)?)?,
        "edit" => edit_file(root, &parse::<EditArgs>(arguments)?)?,
        "grep" => grep(root, &parse::<GrepArgs>(arguments)?)?,
        "bash" => bash(root, &parse::<BashArgs>(arguments)?)?,
        _ => bail!("unknown tool {name}"),
    };
    Ok(bounded(&output))
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    contents: String,
}

#[derive(Debug, Deserialize)]
struct EditArgs {
    path: String,
    old: String,
    new: String,
}

#[derive(Debug, Deserialize)]
struct GrepArgs {
    pattern: String,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
    serde_json::from_str(arguments).context("tool arguments are not valid JSON")
}

fn read_file(root: &Path, args: &ReadArgs) -> Result<String> {
    let path = workspace_file(root, &args.path)?;
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.contains(&0) {
        bail!("{} is not a text file", path.display());
    }
    String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))
}

fn write_file(root: &Path, args: &WriteArgs) -> Result<String> {
    let path = workspace_file(root, &args.path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, &args.contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(format!("wrote {}", args.path))
}

fn edit_file(root: &Path, args: &EditArgs) -> Result<String> {
    if args.old.is_empty() {
        bail!("old text is empty");
    }
    let path = workspace_file(root, &args.path)?;
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let count = text.matches(&args.old).count();
    if count != 1 {
        bail!("expected one occurrence of the old text, found {count}");
    }
    let updated = text.replacen(&args.old, &args.new, 1);
    fs::write(&path, updated).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(format!("edited {}", args.path))
}

fn grep(root: &Path, args: &GrepArgs) -> Result<String> {
    let pattern = Regex::new(&args.pattern).context("invalid regular expression")?;
    let start = match args.path.as_deref() {
        None | Some(".") => root.to_path_buf(),
        Some(path) => workspace_file(root, path)?,
    };
    let mut hits = Vec::new();
    collect(root, &start, &pattern, &mut hits)?;
    if hits.is_empty() {
        return Ok(String::new());
    }
    Ok(hits.join("\n"))
}

fn collect(root: &Path, path: &Path, pattern: &Regex, hits: &mut Vec<String>) -> Result<()> {
    if hits.len() >= MATCH_LIMIT || is_symlink(path) {
        return Ok(());
    }
    if path.is_dir() {
        let entries =
            fs::read_dir(path).with_context(|| format!("failed to list {}", path.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("failed to list {}", path.display()))?;
            let name = entry.file_name();
            if name == ".git" || name == ".omp" || is_symlink(&entry.path()) {
                continue;
            }
            collect(root, &entry.path(), pattern, hits)?;
        }
        return Ok(());
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.contains(&0) {
        return Ok(());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(());
    };
    for (index, line) in text.lines().enumerate() {
        if !pattern.is_match(line) {
            continue;
        }
        let number = index.saturating_add(1);
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        hits.push(format!("{relative}:{number}:{line}"));
        if hits.len() >= MATCH_LIMIT {
            break;
        }
    }
    Ok(())
}

fn bash(root: &Path, args: &BashArgs) -> Result<String> {
    if args.command.is_empty() {
        bail!("command is empty");
    }
    let output = command(root, &args.command)
        .output()
        .with_context(|| format!("failed to start {}", args.command))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        return Ok(text);
    }
    let code = output.status.code().unwrap_or(1);
    Ok(format!("{text}\nexit {code}"))
}

fn command(root: &Path, command_text: &str) -> Command {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        let _args: &mut Command = command.arg("/C").arg(command_text);
        command
    } else {
        let mut command = Command::new("sh");
        let _args: &mut Command = command.arg("-c").arg(command_text);
        command
    };
    let _directory: &mut Command = command.current_dir(root);
    command
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn bounded(text: &str) -> String {
    let mut out = String::new();
    for (index, character) in text.chars().enumerate() {
        if index >= OUTPUT_LIMIT {
            out.push_str("\ntruncated");
            break;
        }
        out.push(character);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::invoke;
    use crate::chat::{FunctionCall, ToolCall};

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "1".to_owned(),
            kind: "function".to_owned(),
            function: FunctionCall {
                name: name.to_owned(),
                arguments: arguments.to_owned(),
            },
        }
    }

    #[test]
    fn write_read_edit_and_grep_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let wrote = invoke(
            root.path(),
            &call("write", r#"{"path":"src/a.txt","contents":"alpha beta"}"#),
        )
        .unwrap();
        assert!(wrote.contains("wrote"));
        let read = invoke(root.path(), &call("read", r#"{"path":"src/a.txt"}"#)).unwrap();
        assert_eq!(read, "alpha beta");
        let edited = invoke(
            root.path(),
            &call("edit", r#"{"path":"src/a.txt","old":"beta","new":"gamma"}"#),
        )
        .unwrap();
        assert!(edited.contains("edited"));
        let hits = invoke(
            root.path(),
            &call("grep", r#"{"pattern":"gamma","path":"src"}"#),
        )
        .unwrap();
        assert!(hits.contains("alpha gamma"));
    }

    #[test]
    fn edit_requires_one_match_and_paths_cannot_escape() {
        let root = tempfile::tempdir().unwrap();
        let wrote = invoke(
            root.path(),
            &call("write", r#"{"path":"a.txt","contents":"one one"}"#),
        )
        .unwrap();
        assert!(wrote.contains("wrote"), "{wrote}");
        let error = invoke(
            root.path(),
            &call("edit", r#"{"path":"a.txt","old":"one","new":"two"}"#),
        )
        .unwrap_err();
        assert!(error.to_string().contains("one occurrence"));
        let escaped = invoke(root.path(), &call("read", r#"{"path":"../secret"}"#)).unwrap_err();
        assert!(escaped.to_string().contains("escapes"));
    }

    #[cfg(unix)]
    #[test]
    fn grep_skips_symlinked_directories() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "outside-secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::fs::write(root.path().join("local.txt"), "outside-secret").unwrap();
        let hits = invoke(
            root.path(),
            &call("grep", r#"{"pattern":"outside-secret"}"#),
        )
        .unwrap();
        assert!(hits.contains("local.txt"), "{hits}");
        assert!(!hits.contains("secret.txt"), "{hits}");
    }

    #[cfg_attr(miri, ignore)]
    #[test]
    fn bash_runs_in_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let command = if cfg!(windows) {
            r#"{"command":"echo workspace"}"#
        } else {
            r#"{"command":"printf workspace"}"#
        };
        let output = invoke(root.path(), &call("bash", command)).unwrap();
        assert!(output.contains("workspace"));
    }

    #[test]
    fn unknown_tool_and_bad_json_fail() {
        let root = tempfile::tempdir().unwrap();
        let unknown = invoke(root.path(), &call("nope", "{}")).unwrap_err();
        assert!(unknown.to_string().contains("unknown"), "{unknown}");
        let invalid = invoke(root.path(), &call("read", "not-json")).unwrap_err();
        assert!(invalid.to_string().contains("JSON"), "{invalid}");
    }
}
