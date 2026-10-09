use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::chat::ChatMessage;

pub(crate) fn append(root: &Path, message: &ChatMessage) -> Result<()> {
    let path = root.join(".omp/session.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let line = serde_json::to_string(message).context("failed to encode a session message")?;
    writeln!(file, "{line}").with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::append;
    use crate::chat::user;

    #[test]
    fn messages_append_as_json_lines() {
        let root = tempfile::tempdir().unwrap();
        append(root.path(), &user("one")).unwrap();
        append(root.path(), &user("two")).unwrap();
        let text = fs::read_to_string(root.path().join(".omp/session.jsonl")).unwrap();
        assert!(text.contains("\"content\":\"one\""));
        assert!(text.contains("\"content\":\"two\""));
        assert_eq!(text.lines().count(), 2);
    }
}
