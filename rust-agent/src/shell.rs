use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) home: PathBuf,
    pub(crate) data: PathBuf,
    pub(crate) state: PathBuf,
    pub(crate) executable: PathBuf,
}

pub(crate) fn auto_install() {
    if cfg!(debug_assertions)
        || env::var_os("OMP_SKIP_INTEGRATIONS").is_some()
        || already_installed()
    {
        return;
    }
    match install_from_process() {
        Ok(()) => {}
        Err(error) => {
            let _error = error;
        }
    }
}

pub(crate) fn install(shell: &str, layout: &Layout, script: &str) -> Result<PathBuf> {
    let script_path = script_path(shell, layout)?;
    write_file(&script_path, script.as_bytes())?;
    place_shortcut(&layout.executable)?;
    write_file(&layout.state.join("shortcut-installed"), b"installed\n")?;
    write_file(
        &layout.state.join(format!("{shell}-installed")),
        b"installed\n",
    )?;
    Ok(script_path)
}

pub(crate) fn detected_shell() -> Option<String> {
    env::var_os("SHELL").and_then(|value| {
        Path::new(&value)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_ascii_lowercase)
    })
}

fn already_installed() -> bool {
    let Some(home) = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        return false;
    };
    let state = env::var_os("XDG_STATE_HOME")
        .map_or_else(|| home.join(".local/state"), PathBuf::from)
        .join("omp");
    let shell = detected_shell().unwrap_or_else(|| "bash".to_owned());
    fs::metadata(state.join(format!("{shell}-installed"))).is_ok()
}

fn install_from_process() -> Result<()> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let data =
        env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from);
    let state = env::var_os("XDG_STATE_HOME")
        .map_or_else(|| home.join(".local/state"), PathBuf::from)
        .join("omp");
    let executable = current_executable()?;
    let shell = detected_shell().unwrap_or_else(|| "bash".to_owned());
    let script = crate::cli::completion_script(&shell)?;
    let layout = Layout {
        home,
        data,
        state,
        executable,
    };
    drop(install(&shell, &layout, &script)?);
    Ok(())
}

fn script_path(shell: &str, layout: &Layout) -> Result<PathBuf> {
    let path = match shell {
        "bash" => layout.data.join("bash-completion/completions/omp"),
        "zsh" => layout.home.join(".zfunc/_omp"),
        "fish" => layout.home.join(".config/fish/completions/omp.fish"),
        "powershell" | "pwsh" => layout.home.join("omp-completions.ps1"),
        "elvish" => layout.home.join(".config/elvish/lib/omp.elv"),
        _ => bail!("unsupported shell {shell}"),
    };
    Ok(path)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn place_shortcut(executable: &Path) -> Result<()> {
    let parent = executable
        .parent()
        .context("the omp executable has no parent directory")?;
    let shortcut = parent.join(shortcut_name());
    if same_shortcut(&shortcut, executable) {
        return Ok(());
    }
    if shortcut.symlink_metadata().is_ok() {
        bail!("refusing to replace {}", shortcut.display());
    }
    link(executable, &shortcut)
}

fn same_shortcut(shortcut: &Path, executable: &Path) -> bool {
    if shortcut.symlink_metadata().is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        fs::read_link(shortcut).ok().as_deref() == Some(executable)
    }
    #[cfg(windows)]
    {
        fs::read_to_string(shortcut).unwrap_or_default() == shortcut_body(executable)
    }
}

const fn shortcut_name() -> &'static str {
    if cfg!(windows) {
        "harvest.cmd"
    } else {
        "harvest"
    }
}

#[cfg(unix)]
fn link(executable: &Path, shortcut: &Path) -> Result<()> {
    std::os::unix::fs::symlink(executable, shortcut).with_context(|| {
        format!(
            "failed to link {} to {}",
            shortcut.display(),
            executable.display()
        )
    })
}

#[cfg(windows)]
fn link(executable: &Path, shortcut: &Path) -> Result<()> {
    fs::write(shortcut, shortcut_body(executable))
        .with_context(|| format!("failed to write {}", shortcut.display()))
}

#[cfg(windows)]
fn shortcut_body(executable: &Path) -> String {
    format!("@echo off\r\n\"{}\" %*\r\n", executable.display())
}

fn current_executable() -> Result<PathBuf> {
    // nosemgrep: rust.lang.security.current-exe.current-exe
    env::current_exe().context("failed to locate the omp executable")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Layout, install};

    #[test]
    fn bash_install_writes_the_completion_state_and_shortcut() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("bin/omp");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "bin").unwrap();
        let layout = Layout {
            home: root.path().join("home"),
            data: root.path().join("data"),
            state: root.path().join("state"),
            executable: executable.clone(),
        };
        let written = install("bash", &layout, "complete omp\n").unwrap();
        assert_eq!(
            written,
            root.path().join("data/bash-completion/completions/omp")
        );
        assert_eq!(fs::read_to_string(&written).unwrap(), "complete omp\n");
        assert_eq!(
            fs::read_to_string(root.path().join("state/bash-installed")).unwrap(),
            "installed\n"
        );
        let length = fs::metadata(root.path().join("state/shortcut-installed"))
            .unwrap()
            .len();
        assert!(length > 0, "shortcut state should not be empty");
        drop(install("bash", &layout, "complete omp\n").unwrap());
        #[cfg(unix)]
        {
            assert_eq!(
                fs::read_link(root.path().join("bin/harvest")).unwrap(),
                executable
            );
        }
        #[cfg(windows)]
        {
            assert!(
                fs::read_to_string(root.path().join("bin/harvest.cmd"))
                    .unwrap()
                    .contains("omp")
            );
        }
    }

    #[test]
    fn shell_marker_stays_absent_when_the_shortcut_cannot_be_replaced() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("bin/omp");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "bin").unwrap();
        let blocked = if cfg!(windows) {
            root.path().join("bin/harvest.cmd")
        } else {
            root.path().join("bin/harvest")
        };
        fs::write(&blocked, "other").unwrap();
        let layout = Layout {
            home: root.path().join("home"),
            data: root.path().join("data"),
            state: root.path().join("state"),
            executable,
        };
        let error = install("bash", &layout, "complete omp\n").unwrap_err();
        assert!(error.to_string().contains("refusing"), "{error}");
        drop(fs::metadata(root.path().join("state/bash-installed")).unwrap_err());
    }

    #[test]
    fn unknown_shell_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout {
            home: root.path().join("home"),
            data: root.path().join("data"),
            state: root.path().join("state"),
            executable: root.path().join("omp"),
        };
        let error = install("tcsh", &layout, "script").unwrap_err();
        assert!(error.to_string().contains("unsupported"), "{error}");
    }
}
