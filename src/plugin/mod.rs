pub mod cli;
pub mod host;
pub mod manifest;
pub mod protocol;

use std::path::{Path, PathBuf};

pub use host::{CliSink, NoUiSink, PluginCmdId, PluginCommands, PluginEnv, PluginError, PluginHost, UiSink};
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

impl host::PluginHost {
    /// 설정에서 호스트를 만든다. 매니페스트 문제는 돌려주되 호스트 구성은 계속한다.
    /// CLI 명령은 문제를 무시하고, TUI 시작 화면과 doctor가 보여준다.
    pub fn discover(config: &crate::config::Config) -> (host::PluginHost, Vec<PluginProblem>) {
        let (manifests, problems) = discover(&plugins_dir());
        let host = host::PluginHost::new(
            manifests,
            crate::config::disabled_plugins(config),
            crate::config::plugin_tables(config),
            host::PluginEnv::from_config(config),
        );
        (host, problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_on_a_config_uses_the_plugins_dir_and_never_panics() {
        let config = crate::config::Config::default();
        let (host, problems) = host::PluginHost::discover(&config);
        // 실제 사용자 디렉토리를 읽으므로 내용은 단정하지 않는다. 죽지 않고 일관된 테이블을 만드는지만 본다.
        let _ = problems;
        for (id, c) in host.commands().iter() {
            assert_eq!(host.commands().find(&c.full_name()), Some(id));
        }
    }
}
