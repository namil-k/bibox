//! graph 내장 플러그인. 서재를 citers 입력으로 바꿔 인용 그래프 HTML을 만들고 브라우저로 연다.
//! 데이터는 OpenAlex에서 받아(DOI를 보낸다) `BIBOX_PLUGIN_DIR/cache`에 둔다.

mod input;
mod summary;

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{anyhow, Result};
use clap::Parser;
use serde_json::{json, Value};

use crate::plugin::builtin::Builtin;
use crate::plugin::rpc::RpcError;
use crate::plugin::serve::{serve, Ui};

pub const MANIFEST: &str = r#"api = 2
name = "graph"
description = "Open your library's citation graph in the browser (data from OpenAlex; sends your DOIs to OpenAlex)"

[[commands]]
id = "open"
desc = "Open citation graph"
key = "r"
menus = ["context"]

[[settings]]
key = "min_shared"
type = "int"
default = 3
desc = "Show papers not in library when at least this many library papers cite them"

[[settings]]
key = "mailto"
type = "string"
default = ""
desc = "Email sent to OpenAlex with requests (optional)"

[cli]
run = "builtin"
"#;

pub const GUIDE: &str = "Opens the library's citation graph in the browser: `bibox graph [<key>] [--no-open] [--refresh] [--relayout]`. With a key, that paper is focused (it needs a DOI to be in the graph). With `--no-open`, nothing is opened and stdout is only the path of the HTML file. In the TUI, press `r` on a paper (context menu: Open citation graph). Citation data comes from OpenAlex, so the library's DOIs are sent to OpenAlex; this happens only when you run the command, and it is opt-in. Results are cached under the plugin's `cache` folder (`--refresh` fetches again, `--relayout` forgets saved node positions). Settings under `[plugins.graph]`: `min_shared` (default 3) and `mailto` (optional email for OpenAlex).";

pub const BUILTIN: Builtin = Builtin { name: "graph", manifest: MANIFEST, run, seeded: false, guide: GUIDE };

#[derive(Debug, Clone, PartialEq)]
struct Settings {
    min_shared: usize,
    mailto: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { min_shared: 3, mailto: None }
    }
}

impl Settings {
    fn from_config(c: &Value) -> Settings {
        let min_shared = c.get("min_shared").and_then(Value::as_u64).map(|v| v as usize).unwrap_or(3);
        let mailto = c.get("mailto").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        Settings { min_shared, mailto }
    }
}

#[derive(Debug, Clone)]
struct Paths {
    db: PathBuf,
    notes: PathBuf,
    templates: Option<PathBuf>,
    /// `BIBOX_PLUGIN_DIR`. 캐시와 HTML이 여기 `cache/` 아래 놓인다.
    plugin_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct Flags {
    open_browser: bool,
    refresh: bool,
    relayout: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Flags { open_browser: true, refresh: false, relayout: false }
    }
}

/// 메시지를 만드는 데 필요한 것만 추린 citers 결과(`citers::Output`은 밖에서 못 만든다).
#[derive(Debug, Clone)]
struct GraphResult {
    linked: usize,
    edges: usize,
    failed: Option<String>,
    offline: bool,
    warnings: Vec<String>,
    html_path: PathBuf,
}

impl From<citers::Output> for GraphResult {
    fn from(o: citers::Output) -> Self {
        GraphResult { linked: o.stats.linked, edges: o.stats.edges, failed: o.fetch.failed, offline: o.fetch.offline, warnings: o.warnings, html_path: o.html_path }
    }
}

impl GraphResult {
    fn message(&self) -> String {
        let mut m = format!("Opened citation graph: {} papers linked by {} citations", self.linked, self.edges);
        if let Some(r) = &self.failed {
            m.push_str(&format!("; fetch incomplete: {}", r));
        }
        if self.offline {
            m.push_str("; offline, showing cached data");
        }
        for w in &self.warnings {
            m.push_str("; ");
            m.push_str(w);
        }
        m
    }
}

type Opener<'a> = dyn Fn(&Paths, &Settings, Option<&str>, &mut dyn FnMut(&str)) -> Result<GraphResult> + 'a;

fn join_or_resume<T>(h: std::thread::JoinHandle<T>) -> T {
    match h.join() {
        Ok(v) => v,
        Err(p) => std::panic::resume_unwind(p),
    }
}

fn progress_text(p: &citers::Progress) -> Option<String> {
    match p {
        citers::Progress::Library { done, total } => Some(format!("Fetching citations {}/{}", done, total)),
        citers::Progress::Outside { done, total } => Some(format!("Fetching cited papers {}/{}", done, total)),
        _ => None,
    }
}

fn open_graph(paths: &Paths, settings: &Settings, focus: Option<&str>, flags: Flags, progress: &mut dyn FnMut(&str)) -> Result<GraphResult> {
    let plugin_dir = paths.plugin_dir.as_ref().ok_or_else(|| anyhow!("BIBOX_PLUGIN_DIR is not set; run this through bibox"))?;
    let db = crate::storage::load_db(&paths.db)?;
    let input = input::build_input(&db, &paths.notes, paths.templates.as_deref(), focus);
    let cache = plugin_dir.join("cache");
    let mut opts = citers::Options::default();
    opts.out = Some(cache.join("graph.html"));
    opts.cache_dir = Some(cache);
    opts.open_browser = flags.open_browser;
    opts.refresh = flags.refresh;
    opts.relayout = flags.relayout;
    opts.min_shared = settings.min_shared;
    opts.mailto = settings.mailto.clone();

    // 워커 스레드가 진행을 채널로 보내고, 이 스레드(Ui를 가진 쪽)가 받아서 넘긴다.
    let (tx, rx) = mpsc::channel::<citers::Progress>();
    let worker = std::thread::spawn(move || {
        let mut cb = |p: citers::Progress| {
            let _ = tx.send(p);
        };
        citers::open(&input, &opts, &mut cb)
    });
    for p in rx {
        if let Some(t) = progress_text(&p) {
            progress(&t);
        }
    }
    join_or_resume(worker).map(GraphResult::from)
}

// ── JSON-RPC ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    paths: Option<Paths>,
    settings: Settings,
}

impl State {
    fn handle(&mut self, method: &str, params: Value, ui: &mut Ui, opener: &Opener) -> Result<Value, RpcError> {
        match method {
            "initialize" => {
                let s = |k: &str| params.pointer(&format!("/paths/{}", k)).and_then(Value::as_str).map(PathBuf::from);
                if let (Some(db), Some(notes)) = (s("db"), s("notes")) {
                    self.paths = Some(Paths { db, notes, templates: s("templates"), plugin_dir: env_path("BIBOX_PLUGIN_DIR") });
                }
                self.settings = Settings::from_config(params.get("config").unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            "config/changed" => {
                self.settings = Settings::from_config(params.get("config").unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            "commands/run" => {
                if params.get("command").and_then(Value::as_str) != Some("open") {
                    return Err(RpcError::invalid_params("unknown command"));
                }
                let Some(paths) = &self.paths else { return Err(RpcError::plugin("graph was not initialized")) };
                let focus = params.pointer("/entry/bibtex_key").and_then(Value::as_str).map(str::to_string);
                let mut sink = |t: &str| ui.progress(t);
                let r = opener(paths, &self.settings, focus.as_deref(), &mut sink).map_err(|e| RpcError::plugin(format!("{:#}", e)))?;
                Ok(json!({"message": r.message()}))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn run(args: &[String]) {
    if std::env::var_os("BIBOX_CLI").is_some() {
        std::process::exit(cli_main(args));
    }
    let mut st = State::default();
    let opener = |p: &Paths, s: &Settings, f: Option<&str>, pr: &mut dyn FnMut(&str)| open_graph(p, s, f, Flags::default(), pr);
    let mut handler = |m: &str, p: Value, ui: &mut Ui| -> Result<Value, RpcError> { st.handle(m, p, ui, &opener) };
    serve("graph", &mut handler);
}

// ── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "graph", about = "Open the library's citation graph in the browser")]
struct CliArgs {
    /// Paper to focus (bibtex key)
    key: Option<String>,
    /// Write the HTML but do not open the browser; prints its path
    #[arg(long)]
    no_open: bool,
    /// Fetch everything from OpenAlex again
    #[arg(long)]
    refresh: bool,
    /// Forget saved node positions
    #[arg(long)]
    relayout: bool,
}

fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliArgs, clap::Error> {
    CliArgs::try_parse_from(args)
}

/// `args`는 바깥 clap이 `bibox plugin run graph <args>`에서 파싱한 `<args>`다.
fn cli_main(args: &[String]) -> i32 {
    let a = match parse_cli(std::iter::once("graph".to_string()).chain(args.iter().cloned())) {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    let (Some(db), Some(notes)) = (env_path("BIBOX_DB_PATH"), env_path("BIBOX_NOTES_DIR")) else {
        eprintln!("bibox graph: BIBOX_DB_PATH and BIBOX_NOTES_DIR are not set; run this as `bibox graph`");
        return 1;
    };
    let paths = Paths { db, notes, templates: env_path("BIBOX_TEMPLATES_DIR"), plugin_dir: env_path("BIBOX_PLUGIN_DIR") };
    let settings = env_path("BIBOX_CONFIG_DIR").map(|d| cli_settings(&d.join("config.toml"))).unwrap_or_default();
    let flags = Flags { open_browser: !a.no_open, refresh: a.refresh, relayout: a.relayout };
    let mut shown = false;
    let mut progress = |t: &str| {
        shown = true;
        eprint!("\r{:<40}", t);
    };
    let result = open_graph(&paths, &settings, a.key.as_deref(), flags, &mut progress);
    if shown {
        eprint!("\r{:<40}\r", "");
    }
    match result {
        Ok(r) => {
            for w in &r.warnings {
                eprintln!("Warning: {}", w);
            }
            if let Some(f) = &r.failed {
                eprintln!("Warning: fetch incomplete: {}", f);
            }
            if r.offline {
                eprintln!("Warning: offline, showing cached data");
            }
            if a.no_open {
                println!("{}", r.html_path.display());
            } else {
                println!("Opened citation graph: {} papers linked by {} citations", r.linked, r.edges);
            }
            0
        }
        Err(e) => {
            eprintln!("bibox graph: {:#}", e);
            1
        }
    }
}

/// CLI에는 initialize가 없으므로 `[plugins.graph]`를 config.toml에서 직접 읽는다. 없거나 깨졌으면 기본값.
fn cli_settings(config_toml: &Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(config_toml) else { return Settings::default() };
    let Ok(table) = text.parse::<toml::Table>() else { return Settings::default() };
    let Some(g) = table.get("plugins").and_then(|p| p.get("graph")) else { return Settings::default() };
    serde_json::to_value(g).map(|v| Settings::from_config(&v)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::serve::serve_io;
    use std::io::Cursor;

    /// bibox는 tokio 런타임 안이고 citers는 `reqwest::blocking`을 쓴다. `open_graph`가 스레드를 없애고
    /// `citers::open`을 직접 부르면 여기서 패닉한다. DOI 없는 항목만 있어 네트워크 요청은 없다.
    #[tokio::test]
    async fn open_graph_runs_citers_off_the_runtime() {
        let dir = std::env::temp_dir().join(format!("bibox-graph-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = Paths { db: dir.join("db.json"), notes: dir.join("notes"), templates: None, plugin_dir: Some(dir.join("plugin")) };
        let flags = Flags { open_browser: false, ..Flags::default() };
        let r = open_graph(&p, &Settings::default(), None, flags, &mut |_| {}).unwrap();
        assert_eq!(r.html_path, dir.join("plugin/cache/graph.html"));
        assert!(r.html_path.exists());
    }

    #[test]
    fn cli_args_parse() {
        let a = parse_cli(["graph", "kopper2006design", "--no-open", "--relayout"].map(String::from)).unwrap();
        assert_eq!(a.key.as_deref(), Some("kopper2006design"));
        assert!(a.no_open && a.relayout && !a.refresh);
        let a = parse_cli(["graph"].map(String::from)).unwrap();
        assert!(a.key.is_none() && !a.no_open);
        assert!(parse_cli(["graph", "--bogus"].map(String::from)).is_err());
    }

    fn line(v: Value) -> String {
        format!("{}\n", v)
    }

    fn paths() -> Paths {
        Paths { db: PathBuf::from("/x/db.json"), notes: PathBuf::from("/x/notes"), templates: None, plugin_dir: Some(PathBuf::from("/x/plugin")) }
    }

    fn run_open(opener: &Opener) -> Vec<Value> {
        let input = line(json!({"jsonrpc": "2.0", "id": 1, "method": "commands/run",
            "params": {"command": "open", "entry": {"bibtex_key": "a"}}}));
        let mut out = Vec::new();
        {
            let mut st = State { paths: Some(paths()), settings: Settings::default() };
            let mut h = |m: &str, p: Value, ui: &mut Ui| -> Result<Value, RpcError> { st.handle(m, p, ui, opener) };
            serve_io("graph", &mut Cursor::new(input), &mut out, &mut h);
        }
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn failure_becomes_a_message() {
        let lines = run_open(&|_, _, _, _| Err(anyhow!("could not reach OpenAlex and there is no cache yet")));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["error"]["code"], -32000);
        assert!(lines[0]["error"]["message"].as_str().unwrap().contains("could not reach OpenAlex and there is no cache yet"));
    }

    #[test]
    fn progress_reaches_the_ui() {
        let lines = run_open(&|_, _, focus, progress| {
            assert_eq!(focus, Some("a"));
            progress("Fetching citations 1/3");
            progress("Fetching citations 2/3");
            progress("Fetching cited papers 3/3");
            Ok(GraphResult { linked: 5, edges: 7, failed: Some("rate limited".into()), offline: false, warnings: vec!["w1".into(), "w2".into()], html_path: PathBuf::from("/x/graph.html") })
        });
        assert_eq!(lines.len(), 4);
        for l in &lines[..3] {
            assert_eq!(l["method"], "window/progress");
        }
        assert_eq!(lines[2]["params"]["text"], "Fetching cited papers 3/3");
        assert_eq!(lines[3]["result"]["message"], "Opened citation graph: 5 papers linked by 7 citations; fetch incomplete: rate limited; w1; w2");
    }

    #[test]
    fn progress_text_per_variant() {
        assert_eq!(progress_text(&citers::Progress::Library { done: 50, total: 181 }).as_deref(), Some("Fetching citations 50/181"));
        assert_eq!(progress_text(&citers::Progress::Outside { done: 2, total: 9 }).as_deref(), Some("Fetching cited papers 2/9"));
    }

    #[test]
    fn missing_plugin_dir_is_an_error() {
        let mut p = paths();
        p.plugin_dir = None;
        let e = open_graph(&p, &Settings::default(), None, Flags::default(), &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains("BIBOX_PLUGIN_DIR"));
    }

    #[test]
    fn cli_settings_read_the_plugins_graph_table() {
        let d = std::env::temp_dir().join(format!("bibox-graph-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("config.toml");
        std::fs::write(&f, "[plugins.graph]\nmin_shared = 5\nmailto = \"a@b.c\"\n").unwrap();
        assert_eq!(cli_settings(&f), Settings { min_shared: 5, mailto: Some("a@b.c".into()) });
        assert_eq!(cli_settings(&d.join("nope.toml")), Settings::default());
    }
}
