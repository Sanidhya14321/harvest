use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(crate) fn workspace_file(root: &Path, raw: &str) -> Result<PathBuf> {
    if raw.is_empty() {
        bail!("path is empty");
    }
    let lexical = lexical_join(root, raw)?;
    if lexical == root {
        bail!("path is the workspace root");
    }
    confirm_existing(&lexical, root)?;
    Ok(lexical)
}

fn lexical_join(root: &Path, raw: &str) -> Result<PathBuf> {
    let combined = root.join(raw);
    let mut out = PathBuf::new();
    for component in combined.components() {
        match component {
            Component::ParentDir => {
                if !out.pop() {
                    bail!("path escapes the workspace");
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    if !out.starts_with(root) {
        bail!("path escapes the workspace");
    }
    Ok(out)
}

fn confirm_existing(path: &Path, root: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let Ok(relative) = path.strip_prefix(root) else {
        bail!("path escapes the workspace");
    };
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        let Ok(metadata) = cursor.symlink_metadata() else {
            break;
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let canonical = cursor
            .canonicalize()
            .with_context(|| format!("path escapes the workspace through {}", cursor.display()))?;
        if !canonical.starts_with(&canonical_root) {
            bail!("path escapes the workspace");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;

    use super::workspace_file;

    #[test]
    fn relative_file_stays_inside_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let file = workspace_file(root.path(), "notes/a.txt").unwrap();
        assert_eq!(file, root.path().join("notes/a.txt"));
    }

    #[test]
    fn parent_segments_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let error = workspace_file(root.path(), "../secret").unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn dotted_parent_segments_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let error = workspace_file(root.path(), "nested/../../secret").unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn empty_path_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let error = workspace_file(root.path(), "").unwrap_err();
        assert!(error.to_string().contains("empty"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_that_leaves_the_workspace_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        fs::write(outside.path().join("secret.txt"), "nope").unwrap();
        let error = workspace_file(root.path(), "escape/secret.txt").unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[cfg(unix)]
    #[test]
    fn nested_path_through_a_symlink_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        let error = workspace_file(root.path(), "escape/new/dir/file.txt").unwrap_err();
        assert!(error.to_string().contains("escapes"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp/omp-missing-outside", root.path().join("escape")).unwrap();
        let error = workspace_file(root.path(), "escape").unwrap_err();
        assert!(error.to_string().contains("escapes"), "{error}");
    }
}
