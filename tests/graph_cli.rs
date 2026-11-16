//! `bibox plugin run graph <args>`: 바깥 clap이 파싱한 인자가 graph의 CLI로 그대로 간다. 네트워크와 실제 서재는 건드리지 않는다.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let home = std::env::temp_dir().join(format!("bibox-graph-cli-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bibox"))
        .env_clear()
        .env("HOME", &home)
        .env("BIBOX_CLI", "1")
        .args(["plugin", "run", "graph"])
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
