// `set`, `add` and `restart`: edit config.toml from the command line and
// restart the background service so the change takes effect.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context};

use crate::config::{AccountConfig, Config};
use crate::paths;
use crate::provider::Provider;

fn parse_provider(v: &str) -> anyhow::Result<Provider> {
    match v {
        "claude" => Ok(Provider::Claude),
        "codex" => Ok(Provider::Codex),
        "antigravity" => Ok(Provider::Antigravity),
        _ => bail!("provider must be claude, codex or antigravity"),
    }
}

fn is_none(v: &str) -> bool {
    matches!(v, "none" | "null" | "-" | "")
}

/// `set KEY VALUE`. Top-level keys: interval_s, days, host, email, auto_discover,
/// exclude (comma separated). Account keys: `<label>.display`,
/// `<label>.subscription_usd`, `<label>.provider`, `<label>.path`, `<label>.label`.
/// A label that is only auto-discovered gets an explicit [[accounts]] entry.
pub fn set(cfg: &mut Config, config_path: &Path, home: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    match key {
        "interval_s" => cfg.interval_s = value.parse().context("interval_s must be a number of seconds")?,
        "days" => cfg.days = value.parse().context("days must be a number")?,
        "host" => cfg.host = if is_none(value) { None } else { Some(value.to_string()) },
        "email" => cfg.email = value.to_string(),
        "auto_discover" => cfg.auto_discover = value.parse().context("auto_discover must be true or false")?,
        "exclude" => cfg.exclude = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        _ => {
            let Some((label, field)) = key.rsplit_once('.') else {
                bail!("unknown key {key}; see `claude-usage-collector set --help`");
            };
            let idx = account_index(cfg, home, label)?;
            let a = &mut cfg.accounts[idx];
            match field {
                "display" => a.display = if is_none(value) { None } else { Some(value.to_string()) },
                "subscription_usd" => {
                    a.subscription_usd =
                        if is_none(value) { None } else { Some(value.parse().context("subscription_usd must be a whole number of USD")?) }
                }
                "provider" => a.provider = if is_none(value) { None } else { Some(parse_provider(value)?) },
                "path" => a.path = value.to_string(),
                "label" => a.label = if is_none(value) { None } else { Some(paths::sanitize_label(value)) },
                _ => bail!("unknown account field {field}; use display, subscription_usd, provider, path or label"),
            }
        }
    }
    cfg.save(config_path)?;
    println!("set {key} = {value} in {}", config_path.display());
    Ok(())
}

fn account_index(cfg: &mut Config, home: &Path, label: &str) -> anyhow::Result<usize> {
    let discovered = crate::accounts::discover(home, cfg);
    for (i, a) in cfg.accounts.iter().enumerate() {
        let effective = a.label.clone().or_else(|| {
            let dir = paths::expand(&a.path);
            discovered.iter().find(|d| d.dir == std::fs::canonicalize(&dir).unwrap_or(dir.clone())).map(|d| d.label.clone())
        });
        if effective.as_deref() == Some(label) {
            return Ok(i);
        }
    }
    if let Some(d) = discovered.iter().find(|d| d.label == label) {
        cfg.accounts.push(AccountConfig {
            path: d.dir.to_string_lossy().to_string(),
            provider: Some(d.provider),
            label: Some(d.label.clone()),
            display: None,
            subscription_usd: None,
        });
        return Ok(cfg.accounts.len() - 1);
    }
    let known: Vec<&str> = discovered.iter().map(|d| d.label.as_str()).collect();
    bail!("no account labelled {label}; known: {}. Use `add PATH` for a new one.", known.join(", "));
}

/// `add PATH [--provider P] [--display D] [--label L] [--subscription-usd N]`
pub fn add(cfg: &mut Config, config_path: &Path, path: &str, opts: &[(String, String)]) -> anyhow::Result<()> {
    let mut a = AccountConfig { path: path.to_string(), provider: None, label: None, display: None, subscription_usd: None };
    for (k, v) in opts {
        match k.as_str() {
            "--provider" => a.provider = Some(parse_provider(v)?),
            "--display" => a.display = Some(v.clone()),
            "--label" => a.label = Some(paths::sanitize_label(v)),
            "--subscription-usd" => a.subscription_usd = Some(v.parse().context("--subscription-usd must be a whole number")?),
            _ => bail!("unknown option {k}"),
        }
    }
    let dir = paths::expand(path);
    if a.provider.is_none() && Provider::detect(&dir).is_none() {
        bail!("{}: cannot detect provider (no projects/, sessions/ or antigravity*/conversations/); pass --provider", dir.display());
    }
    cfg.accounts.push(a);
    cfg.save(config_path)?;
    println!("added {path} to {}", config_path.display());
    Ok(())
}

/// Restarts the background service: systemd --user on Linux, the logon task on Windows.
pub fn restart() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let st = Command::new("systemctl").args(["--user", "restart", "claude-usage-collector"]).status()?;
        if !st.success() {
            bail!("systemctl --user restart claude-usage-collector failed (is the service installed? deploy/linux/install.sh)");
        }
        let out = Command::new("systemctl").args(["--user", "is-active", "claude-usage-collector"]).output()?;
        println!("service: {}", String::from_utf8_lossy(&out.stdout).trim());
        return Ok(());
    }
    #[cfg(windows)]
    {
        let _ = Command::new("schtasks").args(["/end", "/tn", "ClaudeUsageCollector"]).output();
        let st = Command::new("schtasks").args(["/run", "/tn", "ClaudeUsageCollector"]).status()?;
        if !st.success() {
            bail!("schtasks /run /tn ClaudeUsageCollector failed (is the task installed? install.ps1)");
        }
        println!("task ClaudeUsageCollector restarted");
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        bail!("restart is only implemented for Linux (systemd --user) and Windows (Task Scheduler)");
    }
}
