use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Claude Code: <dir>/projects/**/*.jsonl
    Claude,
    /// Codex CLI: <dir>/sessions/**/*.jsonl
    Codex,
    /// Antigravity IDE + CLI: <dir>/antigravity*/conversations/*.db
    Antigravity,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Antigravity => "antigravity",
        }
    }

    /// Guess from directory layout; `None` when nothing recognisable is there.
    pub fn detect(dir: &std::path::Path) -> Option<Provider> {
        if dir.join("projects").is_dir() {
            return Some(Provider::Claude);
        }
        if dir.join("sessions").is_dir() {
            return Some(Provider::Codex);
        }
        if dir.join("antigravity").join("conversations").is_dir()
            || dir.join("antigravity-cli").join("conversations").is_dir()
            || dir.join("conversations").is_dir()
        {
            return Some(Provider::Antigravity);
        }
        None
    }
}
