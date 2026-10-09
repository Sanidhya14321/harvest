use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) model: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) max_steps: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Overrides {
    pub(crate) model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) max_steps: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Environment {
    pub(crate) model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) max_steps: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
struct FileSettings {
    model: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    max_steps: Option<u32>,
}

const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MAX_STEPS: u32 = 8;

pub(crate) fn resolve(
    root: &Path,
    flags: &Overrides,
    environment: &Environment,
) -> Result<Settings> {
    let file = read_file(root)?;
    Ok(Settings {
        model: choose(
            flags.model.clone(),
            environment.model.clone(),
            file.model,
            DEFAULT_MODEL.to_owned(),
        ),
        base_url: choose(
            flags.base_url.clone(),
            environment.base_url.clone(),
            file.base_url,
            DEFAULT_BASE_URL.to_owned(),
        ),
        api_key: choose(
            flags.api_key.clone(),
            environment.api_key.clone(),
            file.api_key,
            String::new(),
        ),
        max_steps: checked_steps(
            flags
                .max_steps
                .or(environment.max_steps)
                .or(file.max_steps)
                .unwrap_or(DEFAULT_MAX_STEPS),
        )?,
    })
}

fn checked_steps(steps: u32) -> Result<u32> {
    if steps == 0 {
        bail!("max steps must be at least 1");
    }
    Ok(steps)
}

pub(crate) fn environment_from(read: impl Fn(&str) -> Option<String>) -> Result<Environment> {
    let max_steps = match read("OMP_MAX_STEPS") {
        Some(value) => Some(parse_steps(&value)?),
        None => None,
    };
    Ok(Environment {
        model: read("OMP_MODEL"),
        base_url: read("OMP_BASE_URL"),
        api_key: read("OMP_API_KEY").or_else(|| read("OPENAI_API_KEY")),
        max_steps,
    })
}

fn choose(
    flag: Option<String>,
    environment: Option<String>,
    file: Option<String>,
    default: String,
) -> String {
    flag.or(environment).or(file).unwrap_or(default)
}

fn read_file(root: &Path) -> Result<FileSettings> {
    let path = root.join(".omp/config.toml");
    if !path.exists() {
        return Ok(FileSettings::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn parse_steps(value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .with_context(|| format!("OMP_MAX_STEPS is not a whole number: {value}"))
        .and_then(|steps| {
            if steps == 0 {
                bail!("OMP_MAX_STEPS must be at least 1");
            }
            Ok(steps)
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Overrides, environment_from, resolve};

    #[test]
    fn flags_beat_environment_file_and_defaults() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".omp")).unwrap();
        fs::write(
            root.path().join(".omp/config.toml"),
            "model = \"file-model\"\nbase_url = \"http://file\"\napi_key = \"file-key\"\nmax_steps = 3\n",
        )
        .unwrap();
        let settings = resolve(
            root.path(),
            &Overrides {
                model: Some("flag-model".to_owned()),
                base_url: None,
                api_key: None,
                max_steps: Some(5),
            },
            &super::Environment {
                model: Some("env-model".to_owned()),
                base_url: Some("http://env".to_owned()),
                api_key: Some("env-key".to_owned()),
                max_steps: Some(4),
            },
        )
        .unwrap();
        assert_eq!(settings.model, "flag-model");
        assert_eq!(settings.base_url, "http://env");
        assert_eq!(settings.api_key, "env-key");
        assert_eq!(settings.max_steps, 5);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let root = tempfile::tempdir().unwrap();
        let settings = resolve(
            root.path(),
            &Overrides::default(),
            &super::Environment::default(),
        )
        .unwrap();
        assert_eq!(settings.model, "gpt-4o-mini");
        assert_eq!(settings.base_url, "https://api.openai.com/v1");
        assert_eq!(settings.api_key, "");
        assert_eq!(settings.max_steps, 8);
    }

    #[test]
    fn environment_reader_parses_steps_and_prefers_omp_key() {
        let environment = environment_from(|key| match key {
            "OMP_MODEL" => Some("m".to_owned()),
            "OMP_MAX_STEPS" => Some("2".to_owned()),
            "OMP_API_KEY" => Some("omp".to_owned()),
            "OPENAI_API_KEY" => Some("openai".to_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(environment.model.as_deref(), Some("m"));
        assert_eq!(environment.max_steps, Some(2));
        assert_eq!(environment.api_key.as_deref(), Some("omp"));
    }

    #[test]
    fn zero_steps_are_rejected() {
        let error =
            environment_from(|key| (key == "OMP_MAX_STEPS").then(|| "0".to_owned())).unwrap_err();
        assert!(error.to_string().contains("at least 1"));
    }
}
