// Discovery of Claude Code config dirs (one per account) and their subscription.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::Config;
use crate::paths;

#[derive(Debug, Clone)]
pub struct Account {
    pub label: String,
    pub display: Option<String>,
    pub dir: PathBuf,
    pub subscription: Option<Subscription>,
}

#[derive(Debug, Clone)]
pub struct Subscription {
    pub kind: String,
    pub tier: String,
    pub usd: Option<u32>,
}

impl Account {
    pub fn projects_dir(&self) -> PathBuf {
        self.dir.join("projects")
    }
}

/// Config [[accounts]] first (explicit wins), then — if auto_discover — ~/.claude*
/// dirs with a projects/ subdir and $CLAUDE_CONFIG_DIR. Deduplicated on canonical
/// path, filtered by `exclude`.
pub fn discover(home: &Path, cfg: &Config) -> Vec<Account> {
    let mut out: Vec<Account> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    let mut push = |dir: PathBuf, label: Option<&str>, display: Option<&str>, usd_override: Option<u32>| {
        let canon = fs::canonicalize(&dir).unwrap_or(dir.clone());
        if !seen.insert(canon.clone()) {
            return;
        }
        let label = label
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_label(&canon));
        let label = paths::sanitize_label(&label);
        if cfg.exclude.iter().any(|e| e == &label) {
            return;
        }
        let mut subscription = read_subscription(&canon);
        if let Some(usd) = usd_override {
            let s = subscription.get_or_insert(Subscription { kind: "manual".into(), tier: "manual".into(), usd: None });
            s.usd = Some(usd);
        }
        out.push(Account { label, display: display.map(|s| s.to_string()), dir: canon, subscription });
    };

    for a in &cfg.accounts {
        let dir = paths::expand(&a.path);
        push(dir, a.label.as_deref(), a.display.as_deref(), a.subscription_usd);
    }

    if cfg.auto_discover {
        if let Ok(v) = std::env::var("CLAUDE_CONFIG_DIR") {
            if !v.is_empty() {
                push(paths::expand(&v), None, None, None);
            }
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = fs::read_dir(home) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with(".claude") && p.is_dir() && p.join("projects").is_dir() {
                    candidates.push(p);
                }
            }
        }
        candidates.sort();
        for p in candidates {
            push(p, None, None, None);
        }
    }

    out.retain(|a| {
        let ok = a.projects_dir().is_dir();
        if !ok {
            log::warn!("account {}: {} has no projects/ dir, skipping", a.label, a.dir.display());
        }
        ok
    });
    out
}

fn default_label(dir: &Path) -> String {
    let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "claude".into());
    name.trim_start_matches('.').to_string()
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

/// Reads <dir>/.credentials.json → claudeAiOauth.{subscriptionType, rateLimitTier}.
/// The access/refresh tokens in that file are never read into a struct.
pub fn read_subscription(dir: &Path) -> Option<Subscription> {
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

        let cfg = Config {
            exclude: vec!["claude-test".into()],
            accounts: vec![
                AccountConfig { path: ext.to_string_lossy().to_string(), label: None, display: Some("Work".into()), subscription_usd: Some(100) },
                AccountConfig { path: home.join(".claude").to_string_lossy().to_string(), label: None, display: Some("Private".into()), subscription_usd: None },
            ],
            ..Default::default()
        };
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let accts = discover(home, &cfg);
        let labels: Vec<&str> = accts.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["work", "claude", "claude-5x"]);
        assert_eq!(accts[0].subscription.as_ref().unwrap().usd, Some(100));
        assert_eq!(accts[1].display.as_deref(), Some("Private"));
        assert_eq!(accts[1].subscription.as_ref().unwrap().usd, Some(200));
        assert_eq!(accts[2].subscription.as_ref().unwrap().usd, Some(100));
        assert_eq!(accts[2].display, None);
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
