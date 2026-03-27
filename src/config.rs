use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::i18n::Msgs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub bibox_dir: PathBuf,
    pub pdf_viewer: Option<String>,
    pub default_collection: Option<String>,
    pub search_case_sensitive: bool,
    pub default_page_size: usize,
    #[serde(default = "default_language")]
    pub language: String,
    /// Auto-commit the database to git after every write (default: false)
    #[serde(default)]
    pub git: bool,
    /// Directory for per-entry note files (default: ~/.local/share/bibox/notes/)
    #[serde(default = "default_notes_dir")]
    pub notes_dir: PathBuf,
    #[serde(default = "default_templates_dir")]
    pub templates_dir: PathBuf,
    #[serde(skip)]
    pub msgs: Msgs,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bibox_dir: default_bibox_dir(),
            pdf_viewer: None,
            default_collection: None,
            search_case_sensitive: false,
            default_page_size: 20,
            language: default_language(),
            git: false,
            notes_dir: default_notes_dir(),
            templates_dir: default_templates_dir(),
            msgs: Msgs::default(),
        }
    }
}

fn default_language() -> String {
    "en".to_string()
}

fn default_notes_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        })
        .join("bibox")
        .join("notes")
}

fn default_templates_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bibox")
        .join("templates")
}

fn default_bibox_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("bibox")
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bibox")
        .join("config.toml")
}

pub fn db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bibox")
        .join("db.json")
}

pub fn load_config() -> Result<Config> {
    let path = config_path();
    let mut config = if !path.exists() {
        Config::default()
    } else {
        let content = std::fs::read_to_string(&path)?;
        toml::from_str(&content)?
    };
    config.msgs = Msgs::new(&config.language);
    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}
