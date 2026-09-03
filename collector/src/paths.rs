use std::path::PathBuf;

pub const APP_DIR: &str = "claude-usage-collector";

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Linux: ~/.config/claude-usage-collector. Windows: %APPDATA%\claude-usage-collector.
pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(home_dir).join(APP_DIR)
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_file() -> PathBuf {
    config_dir().join("state.json")
}

pub fn log_file() -> PathBuf {
    config_dir().join("collector.log")
}

/// Expands a leading `~` and `$VAR` / `%VAR%` references.
pub fn expand(p: &str) -> PathBuf {
    let mut s = p.to_string();
    if let Some(rest) = s.strip_prefix("~") {
        s = format!("{}{}", home_dir().display(), rest);
    }
    // $VAR and ${VAR}
    while let Some(start) = s.find('$') {
        let after = &s[start + 1..];
        let (name, end) = if let Some(inner) = after.strip_prefix('{') {
            let close = inner.find('}').map(|i| i + 2).unwrap_or(after.len());
            (inner[..close.saturating_sub(2)].to_string(), start + 1 + close)
        } else {
            let n: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let len = n.len();
            (n, start + 1 + len)
        };
        if name.is_empty() {
            break;
        }
        let val = std::env::var(&name).unwrap_or_default();
        s.replace_range(start..end, &val);
    }
    // %VAR%
    while let (Some(a), Some(b)) = (s.find('%'), s[s.find('%').map(|i| i + 1).unwrap_or(0)..].find('%')) {
        let name = s[a + 1..a + 1 + b].to_string();
        let val = std::env::var(&name).unwrap_or_default();
        s.replace_range(a..a + b + 2, &val);
    }
    PathBuf::from(s)
}

pub fn hostname() -> String {
    hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "unknown-host".into())
}

/// Firestore document ids may not contain '/'; keep labels simple.
pub fn sanitize_label(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect()
}
