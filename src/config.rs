use std::{fmt, fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "config.toml";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum QboEnvironment {
    Sandbox,
    Production,
}

impl QboEnvironment {
    pub fn api_base_url(self) -> &'static str {
        match self {
            Self::Sandbox => "https://sandbox-quickbooks.api.intuit.com",
            Self::Production => "https://quickbooks.api.intuit.com",
        }
    }
}

impl fmt::Display for QboEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sandbox => write!(f, "sandbox"),
            Self::Production => write!(f, "production"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct QboConfig {
    pub environment: QboEnvironment,
    #[serde(default)]
    pub auth_mode: AuthMode,
    pub redirect_uri: String,
    pub client_id: Option<String>,
    #[serde(skip_serializing)]
    pub realm_id: Option<String>,
    pub broker_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    #[default]
    Direct,
    Broker,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::Broker => write!(f, "broker"),
        }
    }
}

impl Default for QboConfig {
    fn default() -> Self {
        Self {
            environment: QboEnvironment::Sandbox,
            auth_mode: AuthMode::Direct,
            redirect_uri: "http://localhost:8734/callback".to_string(),
            client_id: None,
            realm_id: None,
            broker_url: None,
        }
    }
}

impl QboConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if config.realm_id.is_some() {
            // Migrate earlier configuration files into the platform secret store.
            config.save()?;
        } else {
            config.realm_id = crate::secrets::get_realm_id()?;
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        if let Some(realm_id) = &self.realm_id {
            crate::secrets::set_realm_id(realm_id)?;
        }
        let path = config_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn remove() -> Result<()> {
        let path = config_path()?;
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_identifier_is_never_serialized_to_plaintext_config() {
        let config = QboConfig {
            realm_id: Some("sandbox-company-identifier".into()),
            ..QboConfig::default()
        };
        let encoded = toml::to_string(&config).unwrap();
        assert!(!encoded.contains("realm_id"));
        assert!(!encoded.contains("sandbox-company-identifier"));
    }

    #[test]
    fn legacy_company_identifier_can_be_read_for_migration() {
        let legacy = "environment = 'sandbox'\nredirect_uri = 'http://localhost:8734/callback'\nrealm_id = '12345'\n";
        let config: QboConfig = toml::from_str(legacy).unwrap();
        assert_eq!(config.realm_id.as_deref(), Some("12345"));
    }
}

pub fn config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("could not determine user config directory")?;
    Ok(config_dir.join("qbo").join(CONFIG_FILE))
}
