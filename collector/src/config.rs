use std::fs;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub api_key: String,
    pub project_id: String,
    pub email: String,
    /// Only used for the first login; removed from the file afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default = "default_interval")]
    pub interval_s: u64,
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default = "default_true")]
    pub auto_discover: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<AccountConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_usd: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: String::new(),
            project_id: String::new(),
            email: String::new(),
            password: None,
            interval_s: default_interval(),
            days: default_days(),
            host: None,
            auto_discover: true,
            exclude: Vec::new(),
            accounts: Vec::new(),
        }
    }
}

fn default_interval() -> u64 {
    300
}
fn default_days() -> u32 {
    3
}
fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        let s = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn host(&self) -> String {
        crate::paths::sanitize_label(&self.host.clone().unwrap_or_else(crate::paths::hostname))
    }
}

pub const EXAMPLE: &str = r#"# Firebase web API key and project id (both public by design; security is in Firestore rules).
api_key    = "AIzaSyDWc8AdeuuvYPjY0i12TajgsY5uJjKGZmQ"
project_id = "claude-usage-collector-fm"
email      = "you@example.com"
# password = "..."       # optional: used once, then removed. Prefer `claude-usage-collector login`.
interval_s = 300         # seconds between pushes
days       = 3           # how many local days are re-pushed each run
# host = "my-pc"         # default: OS hostname
# exclude = ["claude-test"]

# Auto-discovery finds ~/.claude* dirs containing projects/ plus $CLAUDE_CONFIG_DIR.
# Set to false to only use the [[accounts]] list below.
auto_discover = true

# Any number of accounts, anywhere on disk. `label` is the Firestore key
# (default: dir name without leading dot), `display` is the dashboard name.
# [[accounts]]
# path    = "~/.claude"
# display = "Max 20x (private)"
#
# [[accounts]]
# path    = "~/.claude-5x"
# display = "Max 5x (work)"
#
# [[accounts]]
# path             = 'D:\claude-profiles\client-x'
# label            = "clientx"
# display          = "Client X"
# subscription_usd = 100   # override when .credentials.json is missing or tier unknown
"#;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct State {
    pub uid: Option<String>,
    pub refresh_token: Option<String>,
}

impl State {
    pub fn load(path: &Path) -> State {
        fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
