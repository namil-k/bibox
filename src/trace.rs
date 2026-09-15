//! `BIBOX_TRACE=<파일>`이면 타이밍 줄을 그 파일에 덧붙인다. 안 켜면 비용 0에 가깝다.
//! 미리보기 탭이 왜 느린지 사용자 환경에서 잴 때 쓴다(2026-09-15).

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static SINK: OnceLock<Option<(Mutex<std::fs::File>, Instant)>> = OnceLock::new();

fn sink() -> &'static Option<(Mutex<std::fs::File>, Instant)> {
    SINK.get_or_init(|| {
        let path = std::env::var("BIBOX_TRACE").ok()?;
        let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some((Mutex::new(f), Instant::now()))
    })
}

/// 프로세스 시작부터의 ms와 메시지 한 줄.
pub fn log(msg: impl FnOnce() -> String) {
    if let Some((f, t0)) = sink() {
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if let Ok(mut f) = f.lock() {
            let _ = writeln!(f, "{:10.1} {}", ms, msg());
        }
    }
}
