use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::i18n::Msgs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LineNumbers {
    #[default]
    Absolute,
    Relative,
    None,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Portable home directory (set by `bibox init`). When set, db/pdfs/notes live here.
    #[serde(default)]
    pub home: Option<PathBuf>,
    /// Custom PDF storage directory (e.g. iCloud, Google Drive, Dropbox).
    /// Overrides `home/pdfs/` when set.
    #[serde(default)]
    pub pdf_dir: Option<PathBuf>,
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
    /// Natural scrolling (like macOS default): scroll down to move content up
    #[serde(default)]
    pub natural_scroll: bool,
    /// Show the hint bar at the bottom of the TUI (default: true).
    /// The searchable help overlay covers the full list, so the bar only carries
    /// panel navigation and the few actions used many times a day.
    #[serde(default = "default_true")]
    pub status_bar: bool,
    /// `[plugins.<name>]` 테이블. bibox는 `enabled`만 해석하고 나머지는 플러그인에 그대로 넘긴다.
    /// TOML은 단순 값이 테이블보다 앞에 와야 하므로 마지막 필드다.
    #[serde(default)]
    pub plugins: BTreeMap<String, toml::Table>,
    #[serde(skip)]
    pub msgs: Msgs,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            home: None,
            pdf_dir: None,
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
            natural_scroll: false,
            status_bar: true,
            plugins: BTreeMap::new(),
            msgs: Msgs::default(),
        }
    }
}

fn default_true() -> bool {
    true
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

    // pdf_dir overrides bibox_dir when explicitly set
    if let Some(ref pdf_dir) = config.pdf_dir {
        config.bibox_dir = expand_tilde(pdf_dir);
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

/// `[plugins.x] enabled = false`인 x들.
pub fn disabled_plugins(config: &Config) -> HashSet<String> {
    config
        .plugins
        .iter()
        .filter(|(_, t)| t.get("enabled").and_then(|v| v.as_bool()) == Some(false))
        .map(|(name, _)| name.clone())
        .collect()
}

/// 플러그인에 넘길 JSON. `enabled`는 여기서 빼지 않고 `PluginHost::context`가 뺀다
/// (list 화면이 `enabled`를 봐야 하므로 원본은 유지).
pub fn plugin_tables(config: &Config) -> BTreeMap<String, serde_json::Value> {
    config
        .plugins
        .iter()
        .filter_map(|(name, t)| serde_json::to_value(t).ok().map(|v| (name.clone(), v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_tables_survive_a_config_round_trip() {
        let text = "bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\n\n[plugins.summarize]\nmodel = \"claude-opus-5\"\nmax_tokens = 800\n\n[plugins.entry-tidy]\nenabled = false\n";
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.plugins.len(), 2);
        let out = toml::to_string_pretty(&config).unwrap();
        let again: Config = toml::from_str(&out).unwrap();
        assert_eq!(again.plugins["summarize"]["model"].as_str(), Some("claude-opus-5"));
        assert_eq!(again.plugins["entry-tidy"]["enabled"].as_bool(), Some(false));
    }

    #[test]
    fn disabled_plugins_reads_only_enabled_false() {
        let text = "bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\n[plugins.a]\nenabled = false\n[plugins.b]\nenabled = true\n[plugins.c]\nmodel = \"x\"\n";
        let config: Config = toml::from_str(text).unwrap();
        let d = disabled_plugins(&config);
        assert!(d.contains("a"));
        assert!(!d.contains("b"));
        assert!(!d.contains("c"));
    }

    #[test]
    fn plugin_tables_convert_to_json_values() {
        let text = "bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\n[plugins.a]\nmodel = \"x\"\nn = 3\nflags = [\"p\", \"q\"]\n";
        let config: Config = toml::from_str(text).unwrap();
        let t = plugin_tables(&config);
        assert_eq!(t["a"], serde_json::json!({"model": "x", "n": 3, "flags": ["p", "q"]}));
    }

    #[test]
    fn a_config_without_a_plugins_table_still_loads() {
        let text = "bibox_dir = \"/tmp/b\"\nsearch_case_sensitive = false\ndefault_page_size = 20\n";
        let config: Config = toml::from_str(text).unwrap();
        assert!(config.plugins.is_empty());
    }
}
