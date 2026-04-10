use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::i18n::Msgs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    Absolute,
    Relative,
    None,
}

impl Default for LineNumbers {
    fn default() -> Self { LineNumbers::Absolute }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Portable home directory (set by `bibox init`). When set, db/pdfs/notes live here.
    #[serde(default)]
    pub home: Option<PathBuf>,
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
    /// Line number display in TUI: absolute, relative, none
    #[serde(default)]
    pub line_numbers: LineNumbers,
    /// Panel width ratio [left, center, right] — values are proportional
    #[serde(default = "default_panel_ratio")]
    pub panel_ratio: [u16; 3],
    /// Export directory for .bib files (default: current directory)
    #[serde(default = "default_bib_export_dir")]
    pub bib_export_dir: PathBuf,
    /// Export directory for other formats (yaml, ris, pdf) (default: ~/Downloads)
    #[serde(default = "default_export_dir")]
    pub export_dir: PathBuf,
    /// Citekey format template. Variables: {author}, {year}, {title}
    #[serde(default = "default_citekey_format")]
    pub citekey_format: String,
    #[serde(skip)]
    pub msgs: Msgs,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            home: None,
            bibox_dir: default_bibox_dir(),
            pdf_viewer: None,
            default_collection: None,
            search_case_sensitive: false,
            default_page_size: 20,
            language: default_language(),
            git: false,
            notes_dir: default_notes_dir(),
            templates_dir: default_templates_dir(),
            line_numbers: LineNumbers::default(),
            panel_ratio: default_panel_ratio(),
            bib_export_dir: default_bib_export_dir(),
            export_dir: default_export_dir(),
            citekey_format: default_citekey_format(),
            msgs: Msgs::default(),
        }
    }
}

fn default_language() -> String {
    "en".to_string()
}

fn default_citekey_format() -> String {
    "{author}{year}{title}".to_string()
}

pub const CITEKEY_PRESETS: &[&str] = &[
    "{author}{year}{title}",
    "{author}_{year}{title}",
    "{author}_{title}_{year}",
    "{author}{year}",
];

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

fn default_panel_ratio() -> [u16; 3] {
    [2, 4, 4]
}

fn default_bib_export_dir() -> PathBuf {
    PathBuf::from(".")
}

fn default_export_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
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

    // When home is set, derive paths from it
    if let Some(ref home) = config.home {
        let home = expand_tilde(home);
        config.bibox_dir = home.join("pdfs");
        config.notes_dir = home.join("notes");
    }

    Ok(config)
}

/// Resolve db_path based on config home (must be called after load_config)
pub fn resolve_db_path(config: &Config) -> PathBuf {
    if let Some(ref home) = config.home {
        expand_tilde(home).join("db.json")
    } else {
        db_path()
    }
}

pub fn expand_tilde(path: &std::path::Path) -> PathBuf {
    if let Ok(s) = path.to_str().ok_or(()) {
        if s.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(&s[2..]);
            }
        }
    }
    path.to_path_buf()
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
