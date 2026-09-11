pub mod manifest;
pub mod protocol;

use std::path::{Path, PathBuf};

pub use manifest::{HookKind, Manifest, PluginProblem};

/// `config.toml`, `keymap.toml`과 같은 디렉토리 아래 `plugins/`.
pub fn plugins_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bibox")
        .join("plugins")
}

/// 디렉토리 이름 알파벳순으로 매니페스트를 읽는다. 파일은 무시하고, `plugin.toml`이
/// 없는 디렉토리는 조용히 건너뛴다(doctor가 따로 보고한다). 디렉토리 자체가 없으면 빈 결과.
pub fn discover(dir: &Path) -> (Vec<Manifest>, Vec<PluginProblem>) {
    let mut manifests = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (manifests, problems);
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let manifest_path = d.join("plugin.toml");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue };
        if let Some(m) = manifest::parse_manifest(&d, &text, &mut problems) {
            manifests.push(m);
        }
    }
    (manifests, problems)
}
