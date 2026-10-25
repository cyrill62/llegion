use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

/// Asks the front-end to redraw within the given delay. Called from background threads.
pub type Repaint = Arc<dyn Fn(Duration) + Send + Sync>;

/// Builds a `Command` that never pops up a console window on Windows.
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs a program to completion and returns its stdout when it exits successfully.
pub fn run_cmd(program: &str, args: &[&str]) -> Option<String> {
    let out = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_duration_secs(secs: i64) -> String {
    if secs < 0 {
        return "expired".into();
    }
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 24 * 365 {
        "forever".into()
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Generates a random API token such as `olk_3f9c…` (32 random bytes, hex encoded).
pub fn generate_token() -> String {
    let mut buf = [0u8; 32];
    if getrandom::fill(&mut buf).is_err() {
        // Extremely unlikely; fall back to a time-based seed rather than failing.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    format!("olk_{hex}")
}

pub fn mask_token(token: &str) -> String {
    if token.len() <= 12 {
        return "•".repeat(token.len());
    }
    format!("{}…{}", &token[..8], &token[token.len() - 4..])
}

/// Context sizes offered by the pickers; 0 = server default.
pub const CONTEXT_PRESETS: [u32; 9] = [2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 0];

pub fn ctx_label(v: u32) -> String {
    match v {
        0 => "Auto (server default)".into(),
        v if v % 1024 == 0 => format!("{}K tokens", v / 1024),
        v => format!("{v} tokens"),
    }
}

/// "8K", "128K", or the raw number when not a multiple of 1024.
pub fn ctx_short(c: u64) -> String {
    if c > 0 && c.is_multiple_of(1024) {
        format!("{}K", c / 1024)
    } else {
        c.to_string()
    }
}
