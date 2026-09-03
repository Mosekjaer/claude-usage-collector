// Discovery of Claude Code config dirs (one per account) and their subscription.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::Config;
use crate::paths;
use crate::provider::Provider;

#[derive(Debug, Clone)]
pub struct Account {
    pub label: String,
    pub display: Option<String>,
    pub dir: PathBuf,
    pub provider: Provider,
    pub subscription: Option<Subscription>,
}

#[derive(Debug, Clone)]
pub struct Subscription {
    pub kind: String,
    pub tier: String,
    pub usd: Option<u32>,
}

impl Account {
    /// Directories that hold this provider's data files.
    pub fn data_roots(&self) -> Vec<PathBuf> {
        match self.provider {
            Provider::Claude => vec![self.dir.join("projects")],
            Provider::Codex => vec![self.dir.join("sessions")],
            Provider::Antigravity => crate::antigravity::data_roots(&self.dir),
        }
    }

    pub fn file_ext(&self) -> &'static str {
        match self.provider {
            Provider::Claude | Provider::Codex => "jsonl",
            Provider::Antigravity => "db",
        }
    }

    pub fn parse_fn(&self) -> crate::stats::ParseFn {
        match self.provider {
            Provider::Claude => crate::claude::parse_file,
            Provider::Codex => crate::codex::parse_file,
            Provider::Antigravity => crate::antigravity::parse_db,
        }
    }
}

/// Config [[accounts]] first (explicit wins), then — if auto_discover — ~/.claude*
/// dirs with a projects/ subdir and $CLAUDE_CONFIG_DIR. Deduplicated on canonical
/// path, filtered by `exclude`.
pub fn discover(home: &Path, cfg: &Config) -> Vec<Account> {
    let mut out: Vec<Account> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    let mut push = |dir: PathBuf, provider: Option<Provider>, label: Option<&str>, display: Option<&str>, usd_override: Option<u32>| {
        let canon = fs::canonicalize(&dir).unwrap_or(dir.clone());
        if !seen.insert(canon.clone()) {
            return;
        }
        let Some(provider) = provider.or_else(|| Provider::detect(&canon)) else {
            log::warn!("{}: cannot tell which provider this is (no projects/, sessions/ or antigravity*/conversations/); set `provider`", canon.display());
            return;
        };
        let label = label.map(|s| s.to_string()).unwrap_or_else(|| default_label(&canon, provider));
        let label = paths::sanitize_label(&label);
        if cfg.exclude.iter().any(|e| e == &label) {
            return;
        }
        let mut subscription = read_subscription(&canon, provider);
        if let Some(usd) = usd_override {
            let s = subscription.get_or_insert(Subscription { kind: "manual".into(), tier: "manual".into(), usd: None });
            s.usd = Some(usd);
        }
        out.push(Account { label, display: display.map(|s| s.to_string()), dir: canon, provider, subscription });
    };

    for a in &cfg.accounts {
        let dir = paths::expand(&a.path);
        push(dir, a.provider, a.label.as_deref(), a.display.as_deref(), a.subscription_usd);
    }

    if cfg.auto_discover {
        if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
            if !v.is_empty() {
                push(paths::expand(&v), Some(Provider::Claude), None, None, None);
            }
        }
        let mut candidates: Vec<(PathBuf, Provider)> = Vec::new();
        if let Ok(rd) = fs::read_dir(home) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if !p.is_dir() {
                    continue;
                }
                if name.starts_with(".claude") && p.join("projects").is_dir() {
                    candidates.push((p, Provider::Claude));
                } else if name == ".codex" && p.join("sessions").is_dir() {
                    candidates.push((p, Provider::Codex));
                } else if name == ".gemini"
                    && (p.join("antigravity").join("conversations").is_dir() || p.join("antigravity-cli").join("conversations").is_dir())
                {
                    candidates.push((p, Provider::Antigravity));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        for (p, prov) in candidates {
            push(p, Some(prov), None, None, None);
        }
    }

    out.retain(|a| {
        let ok = a.data_roots().iter().any(|r| r.is_dir());
        if !ok {
            log::warn!("account {}: no data dir under {} for provider {}, skipping", a.label, a.dir.display(), a.provider.as_str());
        }
        ok
    });
    out
}

fn default_label(dir: &Path, provider: Provider) -> String {
    let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = name.trim_start_matches('.').to_string();
    match (provider, name.as_str()) {
        (Provider::Antigravity, "gemini") | (Provider::Antigravity, "") => "antigravity".into(),
        (Provider::Codex, "") => "codex".into(),
        (Provider::Claude, "") => "claude".into(),
        _ => name,
    }
}

#[derive(Deserialize)]
struct CredFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<Oauth>,
}

#[derive(Deserialize)]
struct Oauth {
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

pub fn read_subscription(dir: &Path, provider: Provider) -> Option<Subscription> {
    match provider {
        Provider::Claude => read_claude_subscription(dir),
        Provider::Codex => {
            let plan = crate::codex::read_plan_type(&dir.join("sessions"))?;
            let usd = crate::codex::plan_usd(&plan);
            Some(Subscription { kind: "chatgpt".into(), tier: plan, usd })
        }
        // Google AI Pro/Ultra: nothing on disk says which; set subscription_usd in config.
        Provider::Antigravity => None,
    }
}

/// Reads <dir>/.credentials.json → claudeAiOauth.{subscriptionType, rateLimitTier}.
/// The access/refresh tokens in that file are never read into a struct.
fn read_claude_subscription(dir: &Path) -> Option<Subscription> {
    let s = fs::read_to_string(dir.join(".credentials.json")).ok()?;
    let f: CredFile = serde_json::from_str(&s).ok()?;
    let o = f.oauth?;
    let kind = o.subscription_type?;
    let tier = o.rate_limit_tier.unwrap_or_default();
    let usd = subscription_usd(&kind, &tier);
    Some(Subscription { kind, tier, usd })
}

/// Claude Pro $20, Max 5× $100, Max 20× $200. Unknown → None (dashboard shows "?").
pub fn subscription_usd(kind: &str, tier: &str) -> Option<u32> {
    match kind {
        "max" if tier.contains("20x") => Some(200),
        "max" if tier.contains("5x") => Some(100),
        "pro" => Some(20),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AccountConfig;

    fn mk(dir: &Path, cred: Option<&str>) {
        fs::create_dir_all(dir.join("projects")).unwrap();
        if let Some(c) = cred {
            fs::write(dir.join(".credentials.json"), c).unwrap();
        }
    }

    #[test]
    fn discovers_home_dirs_and_explicit_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        mk(&home.join(".claude"), Some(r#"{"claudeAiOauth":{"accessToken":"x","subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#));
        mk(&home.join(".claude-5x"), Some(r#"{"claudeAiOauth":{"subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}"#));
        mk(&home.join(".claude-test"), None);
        fs::create_dir_all(home.join(".claude-nope")).unwrap(); // no projects/ → ignored
        let ext = home.join("elsewhere").join("work");
        mk(&ext, None);
        fs::create_dir_all(home.join(".codex").join("sessions")).unwrap();
        fs::create_dir_all(home.join(".gemini").join("antigravity").join("conversations")).unwrap();
        fs::create_dir_all(home.join(".gemini").join("antigravity-cli").join("conversations")).unwrap();

        let cfg = Config {
            exclude: vec!["claude-test".into()],
            accounts: vec![
                AccountConfig { path: ext.to_string_lossy().to_string(), provider: None, label: None, display: Some("Work".into()), subscription_usd: Some(100) },
                AccountConfig { path: home.join(".claude").to_string_lossy().to_string(), provider: None, label: None, display: Some("Private".into()), subscription_usd: None },
                AccountConfig { path: home.join(".gemini").to_string_lossy().to_string(), provider: Some(Provider::Antigravity), label: None, display: Some("Google AI Pro".into()), subscription_usd: Some(0) },
            ],
            ..Default::default()
        };
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let accts = discover(home, &cfg);
        let labels: Vec<&str> = accts.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["work", "claude", "antigravity", "claude-5x", "codex"]);
        assert_eq!(accts[0].subscription.as_ref().unwrap().usd, Some(100));
        assert_eq!(accts[0].provider, Provider::Claude);
        assert_eq!(accts[1].display.as_deref(), Some("Private"));
        assert_eq!(accts[1].subscription.as_ref().unwrap().usd, Some(200));
        assert_eq!(accts[2].provider, Provider::Antigravity);
        assert_eq!(accts[2].subscription.as_ref().unwrap().usd, Some(0));
        assert_eq!(accts[3].subscription.as_ref().unwrap().usd, Some(100));
        assert_eq!(accts[3].display, None);
        assert_eq!(accts[4].provider, Provider::Codex);
        assert!(accts[4].subscription.is_none(), "no rollouts yet -> unknown plan");
    }

    #[test]
    fn tier_mapping() {
        assert_eq!(subscription_usd("max", "default_claude_max_20x"), Some(200));
        assert_eq!(subscription_usd("max", "default_claude_max_5x"), Some(100));
        assert_eq!(subscription_usd("pro", ""), Some(20));
        assert_eq!(subscription_usd("max", "weird"), None);
        assert_eq!(subscription_usd("enterprise", ""), None);
    }
}
