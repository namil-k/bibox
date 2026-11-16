//! `bibox graph <args>`는 `bibox plugin run graph -- <args>`로 간다. `--` 덕분에 `--help` 같은 인자도 바깥 clap이 아니라 graph에 닿는다. 네트워크와 실제 서재는 건드리지 않는다.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let home = std::env::temp_dir().join(format!("bibox-graph-cli-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bibox"))
        .env_clear()
        .env("HOME", &home)
        .env("BIBOX_CLI", "1")
        .args(["plugin", "run", "graph", "--"])
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn help_is_the_graph_help_not_the_plugin_run_help() {
    for flag in ["--help", "-h"] {
        let (code, stdout, _) = run(&[flag]);
        assert_eq!(code, 0);
        assert!(stdout.contains("Usage: graph"), "{}", stdout);
        assert!(!stdout.contains("protocol loop"), "{}", stdout);
    }
}

#[test]
fn flags_reach_graph_and_unknown_ones_are_rejected() {
    // 환경 변수가 없어서 서재 단계에서 멈추지만, 인자 파싱은 통과했다는 뜻이다
    for args in [&["somekey", "--no-open"][..], &["--", "--no-open"][..], &["--refresh", "--relayout"][..]] {
        let (code, _, stderr) = run(args);
        assert_eq!(code, 1, "{:?}: {}", args, stderr);
        assert!(stderr.contains("BIBOX_DB_PATH"), "{:?}: {}", args, stderr);
    }
    let (code, _, stderr) = run(&["--bogus"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unexpected argument '--bogus'"), "{}", stderr);
}

/// 격리 HOME에 graph를 설치하고 진짜 `bibox graph ...`를 부른다.
fn run_installed(args: &[&str]) -> (i32, String, String) {
    let home = std::env::temp_dir().join(format!("bibox-graph-cli-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let bibox = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_bibox"))
            .env_clear()
            .env("HOME", &home)
            .args(args)
            .output()
            .unwrap()
    };
    let lib = home.join("lib");
    assert!(bibox(&["init", lib.to_str().unwrap()]).status.success());
    assert!(bibox(&["plugin", "install", "graph"]).status.success());
    let out = bibox(&[&["graph"][..], args].concat());
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn bibox_graph_help_reaches_graph() {
    for flag in ["--help", "-h"] {
        let (code, stdout, stderr) = run_installed(&[flag]);
        assert_eq!(code, 0, "{}", stderr);
        assert!(stdout.contains("Usage: graph"), "{}", stdout);
        assert!(stdout.contains("--no-open"), "{}", stdout);
    }
}

#[test]
fn plugin_run_help_is_clap_help_again() {
    let home = std::env::temp_dir().join(format!("bibox-graph-cli-help-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bibox"))
        .env_clear()
        .env("HOME", &home)
        .args(["plugin", "run", "git-sync", "--help"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:") && stdout.contains("<NAME>"), "{}", stdout);
}
