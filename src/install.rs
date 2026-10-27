//! Installing the coding agents the launcher starts: presence checks and per-OS install
//! commands (winget, Homebrew, pacman, else each agent's official installer), run in a new
//! terminal window so the user sees the progress and can answer prompts and sudo.

use std::path::PathBuf;
use std::process::Command;

/// Folders searched for the agents, and the PATH of the terminals opened for them.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    dirs.extend(registry_path());
    // shortcut: a few usual install dirs, for GUI sessions started without the shell's PATH; add more if an installer reports elsewhere.
    if !cfg!(windows) {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            dirs.push(home.join(".local/bin"));
            dirs.push(home.join(".opencode/bin"));
            dirs.push(home.join(".pi/agent/bin"));
        }
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    dirs
}

/// The user and system PATH as stored now: winget and the install scripts add to them, but
/// this process keeps the PATH it started with.
#[cfg(windows)]
fn registry_path() -> Vec<PathBuf> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::w;
    let keys = [
        (HKEY_CURRENT_USER, w!("Environment")),
        (
            HKEY_LOCAL_MACHINE,
            w!(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
        ),
    ];
    let mut dirs = Vec::new();
    for (key, subkey) in keys {
        // Big enough for the longest PATH Windows allows; %VARS% come back expanded.
        let mut buf = vec![0u16; 32_768];
        let mut size = (buf.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                key,
                subkey,
                w!("Path"),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status.is_ok() {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            dirs.extend(std::env::split_paths(&String::from_utf16_lossy(
                &buf[..len],
            )));
        }
    }
    dirs
}

/// Agents are identified by their `ollama launch` key, which is also their binary name.
pub fn is_installed(agent: &str) -> bool {
    let names: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd"]
    } else {
        &[""]
    };
    search_dirs().iter().any(|d| {
        names
            .iter()
            .any(|ext| d.join(format!("{agent}{ext}")).is_file())
    })
}

/// One way to install an agent: what it goes through, and the command run in a terminal.
#[derive(Debug, Clone, PartialEq)]
pub struct Method {
    pub via: &'static str,
    pub command: String,
}

/// The install methods of each agent missing from this machine.
pub type Missing = std::collections::HashMap<&'static str, Vec<Method>>;

impl Method {
    fn new(via: &'static str, command: impl Into<String>) -> Self {
        Self {
            via,
            command: command.into(),
        }
    }
}

/// The ways to install `agent` here, recommended first: the system's package manager when it
/// has the agent (apt, dnf and Flathub carry none of them), then the official installer,
/// then npm.
pub fn methods(agent: &str) -> Vec<Method> {
    if cfg!(windows) {
        windows(agent)
    } else if cfg!(target_os = "macos") {
        macos(agent)
    } else {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let aur = ["yay", "paru"].into_iter().find(|h| is_installed(h));
        linux(
            agent,
            os_like(&os_release, "arch"),
            os_like(&os_release, "gentoo"),
            aur,
        )
    }
}

fn npm(agent: &str) -> Option<Method> {
    let package = match agent {
        "claude" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "opencode" => "opencode-ai",
        "pi" => "--ignore-scripts @earendil-works/pi-coding-agent",
        "omp" => "@oh-my-pi/pi-coding-agent",
        "openclaw" => "openclaw",
        "copilot" => "@github/copilot",
        "cline" => "cline",
        "droid" => "droid",
        _ => return None,
    };
    Some(Method::new("npm", format!("npm install -g {package}")))
}

fn windows(agent: &str) -> Vec<Method> {
    let winget = match agent {
        "claude" => Some("Anthropic.ClaudeCode"),
        "codex" => Some("OpenAI.Codex"),
        "opencode" => Some("SST.opencode"),
        "copilot" => Some("GitHub.Copilot"),
        _ => None,
    };
    let script = match agent {
        "claude" => Some("https://claude.ai/install.ps1"),
        "hermes" => Some("https://hermes-agent.nousresearch.com/install.ps1"),
        "omp" => Some("https://omp.sh/install.ps1"),
        "pi" => Some("https://pi.dev/install.ps1"),
        _ => None,
    };
    // Their winget packages are the desktop apps, which may not put the CLI `ollama launch`
    // runs on PATH: offered after npm.
    let desktop = match agent {
        "cline" => Some("Cline.ClineDesktop"),
        "openclaw" => Some("OpenClaw.OpenClawCompanion"),
        _ => None,
    };
    let winget_install = |id| format!("winget install --exact --id {id}");
    winget
        .map(|id| Method::new("winget", winget_install(id)))
        .into_iter()
        .chain(script.map(|url| Method::new("official installer", ps1(url))))
        .chain(npm(agent))
        .chain(desktop.map(|id| Method::new("winget (desktop app)", winget_install(id))))
        .collect()
}

fn macos(agent: &str) -> Vec<Method> {
    // The openclaw cask is the menu bar app, not the CLI.
    let brew = match agent {
        "claude" => Some("--cask claude-code"),
        "codex" => Some("--cask codex"),
        "copilot" => Some("--cask copilot-cli"),
        "droid" => Some("--cask droid"),
        "opencode" => Some("opencode"),
        "cline" => Some("cline"),
        _ => None,
    };
    brew.map(|p| Method::new("Homebrew", format!("brew install {p}")))
        .into_iter()
        .chain(unix_script(agent))
        .chain(npm(agent))
        .collect()
}

fn linux(agent: &str, arch: bool, gentoo: bool, aur: Option<&'static str>) -> Vec<Method> {
    let mut methods = Vec::new();
    if arch {
        let official = match agent {
            "opencode" => Some("opencode"),
            "codex" => Some("openai-codex"),
            _ => None,
        };
        let community = match agent {
            "claude" => Some("claude-code"),
            "copilot" => Some("github-copilot-cli"),
            "cline" => Some("cline-cli"),
            "openclaw" => Some("openclaw"),
            "hermes" => Some("hermes-agent"),
            "omp" => Some("oh-my-pi"),
            _ => None,
        };
        if let Some(p) = official {
            methods.push(Method::new(
                "pacman",
                format!("sudo pacman -S --needed {p}"),
            ));
        }
        if let (Some(helper), Some(p)) = (aur, community) {
            methods.push(Method::new(helper, format!("{helper} -S --needed {p}")));
        }
    }
    if gentoo && agent == "claude" {
        methods.push(Method::new(
            "emerge",
            "sudo emerge --ask dev-util/claude-code",
        ));
    }
    methods.extend(unix_script(agent));
    methods.extend(npm(agent));
    methods
}

fn unix_script(agent: &str) -> Option<Method> {
    let (url, shell) = match agent {
        "claude" => ("https://claude.ai/install.sh", "bash"),
        "opencode" => ("https://opencode.ai/install", "bash"),
        "droid" => ("https://app.factory.ai/cli", "sh"),
        "hermes" => ("https://hermes-agent.nousresearch.com/install.sh", "bash"),
        "omp" => ("https://omp.sh/install", "sh"),
        "pi" => ("https://pi.dev/install.sh", "sh"),
        _ => return None,
    };
    Some(Method::new("official installer", script(url, shell)))
}

fn script(url: &str, shell: &str) -> String {
    format!("curl -fsSL {url} | {shell}")
}

/// `ollama launch <agent>` run from `dir` (the agent works on that folder), against the
/// Ollama server at `host`.
pub fn launch_command(agent: &str, dir: &str, host: &str) -> String {
    if cfg!(windows) {
        launch_powershell(agent, dir, host)
    } else {
        launch_sh(agent, dir, host)
    }
}

fn launch_powershell(agent: &str, dir: &str, host: &str) -> String {
    // Single-quoted PowerShell strings are literal; a quote is escaped by doubling it.
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    format!(
        "Set-Location -LiteralPath {}; $env:OLLAMA_HOST = {}; ollama launch {agent}",
        quote(dir),
        quote(host)
    )
}

fn launch_sh(agent: &str, dir: &str, host: &str) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    format!(
        "cd {} && OLLAMA_HOST={} ollama launch {agent}",
        quote(dir),
        quote(host)
    )
}

/// Windows install scripts, run in the PowerShell window `open_terminal` opens.
fn ps1(url: &str) -> String {
    format!("irm {url} | iex")
}

/// Whether /etc/os-release names `id` as the distro or one it derives from.
fn os_like(os_release: &str, id: &str) -> bool {
    os_release.lines().any(|l| {
        let (key, value) = l.split_once('=').unwrap_or_default();
        matches!(key, "ID" | "ID_LIKE")
            && value.trim_matches('"').split_whitespace().any(|v| v == id)
    })
}

/// Runs `cmd` in a new terminal window that stays open afterwards: PowerShell on Windows,
/// `sh` elsewhere.
pub fn open_terminal(cmd: &str) -> std::io::Result<()> {
    // Lets the agents installed since this app started be found there too.
    let path = std::env::join_paths(search_dirs()).map_err(std::io::Error::other)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        // Bypass: npm's PowerShell shims are blocked by the default execution policy.
        reap(
            Command::new("powershell")
                .env("PATH", path)
                .args(["-NoExit", "-ExecutionPolicy", "Bypass", "-Command", cmd])
                .creation_flags(CREATE_NEW_CONSOLE)
                .spawn()?,
        );
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let quoted = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        reap(
            Command::new("osascript")
                .env("PATH", path)
                .args(["-e", "tell application \"Terminal\" to activate"])
                .args([
                    "-e",
                    &format!("tell application \"Terminal\" to do script \"{quoted}\""),
                ])
                .spawn()?,
        );
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let script = format!("{cmd}; echo; printf 'Press Enter to close. '; read _");
        let preferred = std::env::var("TERMINAL").ok().map(|t| (t, "-e"));
        let known = [
            ("x-terminal-emulator", "-e"),
            ("gnome-terminal", "--"),
            ("konsole", "-e"),
            ("xfce4-terminal", "-x"),
            ("alacritty", "-e"),
            ("kitty", ""),
            ("xterm", "-e"),
        ];
        let candidates = preferred
            .into_iter()
            .chain(known.iter().map(|(t, flag)| (t.to_string(), *flag)));
        for (terminal, flag) in candidates {
            let mut command = Command::new(&terminal);
            command.env("PATH", &path);
            if !flag.is_empty() {
                command.arg(flag);
            }
            match command.args(["sh", "-c", &script]).spawn() {
                Ok(child) => {
                    reap(child);
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no terminal emulator found; set $TERMINAL",
        ))
    }
}

/// Waits for the child in the background so it doesn't linger as a zombie.
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || child.wait());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_commands() {
        let vias = |m: Vec<Method>| m.iter().map(|m| m.via).collect::<Vec<_>>();
        let first = |m: Vec<Method>| m[0].command.clone();
        assert_eq!(
            first(windows("claude")),
            "winget install --exact --id Anthropic.ClaudeCode"
        );
        assert_eq!(
            vias(windows("claude")),
            ["winget", "official installer", "npm"]
        );
        assert_eq!(vias(windows("cline")), ["npm", "winget (desktop app)"]);
        assert_eq!(
            windows("openclaw")[1].command,
            "winget install --exact --id OpenClaw.OpenClawCompanion"
        );
        assert_eq!(vias(windows("hermes")), ["official installer"]);
        assert_eq!(first(windows("pi")), "irm https://pi.dev/install.ps1 | iex");
        assert_eq!(first(macos("cline")), "brew install cline");
        assert_eq!(vias(macos("openclaw")), ["npm"]);
        assert_eq!(
            first(macos("pi")),
            "curl -fsSL https://pi.dev/install.sh | sh"
        );
        assert_eq!(
            vias(linux("codex", true, false, Some("yay"))),
            ["pacman", "npm"]
        );
        assert_eq!(
            first(linux("claude", true, false, Some("paru"))),
            "paru -S --needed claude-code"
        );
        assert_eq!(
            vias(linux("claude", true, false, None)),
            ["official installer", "npm"]
        );
        assert_eq!(
            first(linux("claude", false, true, None)),
            "sudo emerge --ask dev-util/claude-code"
        );
        assert_eq!(
            first(linux("opencode", false, false, None)),
            "curl -fsSL https://opencode.ai/install | bash"
        );
        assert!(linux("unknown", false, false, None).is_empty());
        assert_eq!(
            launch_sh("pi", "/tmp/it's", "http://127.0.0.1:8443"),
            "cd '/tmp/it'\\''s' && OLLAMA_HOST='http://127.0.0.1:8443' ollama launch pi"
        );
        assert_eq!(
            launch_powershell("pi", r"C:\it's", "http://127.0.0.1:8443"),
            r"Set-Location -LiteralPath 'C:\it''s'; $env:OLLAMA_HOST = 'http://127.0.0.1:8443'; ollama launch pi"
        );
        assert!(os_like(
            "NAME=\"EndeavourOS\"\nID=\"endeavouros\"\nID_LIKE=\"arch\"\n",
            "arch"
        ));
        assert!(!os_like("ID=ubuntu\nID_LIKE=debian\n", "arch"));
    }
}
