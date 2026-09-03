#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod accounts;
mod antigravity;
mod auth;
mod claude;
mod codex;
mod config;
mod firestore;
mod logger;
mod paths;
mod proto;
mod provider;
mod stats;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::Local;

use accounts::Account;
use config::{Config, State};
use stats::Scanner;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
claude-usage-collector {VERSION}

USAGE:
  claude-usage-collector [run] [--once] [--backfill N] [--config PATH] [--debug]
  claude-usage-collector login   [--config PATH]      sign in interactively, store refresh token
  claude-usage-collector accounts [--config PATH]     list discovered accounts
  claude-usage-collector scan [--backfill N]          print per-day totals without pushing
  claude-usage-collector init    [--config PATH]      write an example config.toml
  claude-usage-collector paths                        print config/state/log locations

OPTIONS:
  --once          push once and exit (default: loop every interval_s)
  --backfill N    push the last N days instead of config `days`
  --config PATH   config file (default: platform config dir)
  --debug         verbose logging
";

struct Args {
    cmd: String,
    once: bool,
    backfill: Option<u32>,
    config: PathBuf,
    debug: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args { cmd: "run".into(), once: false, backfill: None, config: paths::config_file(), debug: false };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "run" | "login" | "accounts" | "init" | "paths" | "scan" => a.cmd = x,
            "--once" => a.once = true,
            "--debug" => a.debug = true,
            "--backfill" => a.backfill = Some(it.next().context("--backfill needs N")?.parse()?),
            "--config" => a.config = PathBuf::from(it.next().context("--config needs PATH")?),
            "-h" | "--help" => {
                print!("{}", USAGE.replace("{VERSION}", VERSION));
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("{VERSION}");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}\n{}", USAGE.replace("{VERSION}", VERSION)),
        }
    }
    Ok(a)
}

#[cfg(windows)]
fn attach_console() {
    // With windows_subsystem = "windows" there is no console; attach to the
    // parent's so `accounts`, `login`, `paths` print when run from a terminal.
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
#[cfg(not(windows))]
fn attach_console() {}

fn main() {
    attach_console();
    if let Err(e) = real_main() {
        log::error!("{e:#}");
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    let args = parse_args()?;
    match args.cmd.as_str() {
        "paths" => {
            println!("config: {}", paths::config_file().display());
            println!("state:  {}", paths::state_file().display());
            println!("log:    {}", paths::log_file().display());
            return Ok(());
        }
        "init" => {
            if args.config.exists() {
                bail!("{} already exists", args.config.display());
            }
            if let Some(d) = args.config.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&args.config, config::EXAMPLE)?;
            println!("wrote {}", args.config.display());
            return Ok(());
        }
        _ => {}
    }

    logger::init(&paths::log_file(), args.debug);
    let cfg = Config::load(&args.config)
        .with_context(|| format!("no usable config at {} (run `claude-usage-collector init`)", args.config.display()))?;
    let home = paths::home_dir();

    match args.cmd.as_str() {
        "accounts" => {
            let accts = accounts::discover(&home, &cfg);
            print_accounts(&accts, &cfg.host());
            Ok(())
        }
        "scan" => {
            let accts = accounts::discover(&home, &cfg);
            let days = args.backfill.unwrap_or(cfg.days).max(1);
            let from = Local::now().date_naive() - chrono::Duration::days(i64::from(days) - 1);
            for a in &accts {
                let mut sc = Scanner::default();
                let agg = sc.scan(&a.data_roots(), a.file_ext(), from, a.parse_fn())?;
                println!("== {} ({}) files={} parsed={}", a.label, a.provider.as_str(), sc.files_seen_last_scan, sc.files_parsed_last_scan);
                let mut tot: std::collections::BTreeMap<String, stats::ModelTotals> = Default::default();
                for (_, models) in &agg {
                    for (m, t) in models {
                        tot.entry(m.clone()).or_default().add(t);
                    }
                }
                for (m, t) in &tot {
                    println!("  {:<24} replies={:<6} in={:<10} cache_r={:<11} cache_w={:<9} out={}", m, t.replies, t.input, t.cache_read, t.cache_write_5m + t.cache_write_1h, t.output);
                }
                println!("  days with usage: {}", agg.len());
            }
            Ok(())
        }
        "login" => {
            let state_path = paths::state_file();
            let mut state = State::load(&state_path);
            let password = match &cfg.password {
                Some(p) => p.clone(),
                None => rpassword::prompt_password(format!("Password for {}: ", cfg.email))?,
            };
            let s = auth::sign_in_with_password(&cfg.api_key, &cfg.email, &password)?;
            state.uid = Some(s.uid.clone());
            state.refresh_token = Some(s.refresh_token);
            state.save(&state_path)?;
            scrub_password(&cfg, &args.config)?;
            println!("signed in as {} (uid {}); refresh token stored in {}", cfg.email, s.uid, state_path.display());
            Ok(())
        }
        _ => run(&cfg, &args, &home),
    }
}

fn print_accounts(accts: &[Account], host: &str) {
    if accts.is_empty() {
        println!("no accounts found (no ~/.claude*/projects and no [[accounts]] in config)");
        return;
    }
    println!("{:<14} {:<12} {:<22} {:<10} {:<26} {:>6}  path", "label", "provider", "display", "type", "tier", "usd");
    for a in accts {
        let (kind, tier, usd) = match &a.subscription {
            Some(s) => (s.kind.as_str(), s.tier.as_str(), s.usd.map(|u| format!("${u}")).unwrap_or_else(|| "?".into())),
            None => ("-", "-", "?".into()),
        };
        println!(
            "{:<14} {:<12} {:<22} {:<10} {:<26} {:>6}  {}",
            a.label,
            a.provider.as_str(),
            a.display.as_deref().unwrap_or("-"),
            kind,
            tier,
            usd,
            a.dir.display()
        );
    }
    println!("host: {host}");
}

fn scrub_password(cfg: &Config, path: &Path) -> anyhow::Result<()> {
    if cfg.password.is_some() {
        let mut c = cfg.clone();
        c.password = None;
        c.save(path)?;
        log::info!("removed password from {}", path.display());
    }
    Ok(())
}

fn ensure_session(cfg: &Config, config_path: &Path, state: &mut State, cur: Option<auth::Session>) -> anyhow::Result<auth::Session> {
    if let Some(s) = cur {
        if !s.needs_refresh() {
            return Ok(s);
        }
    }
    let s = match (&state.refresh_token, &cfg.password) {
        (Some(rt), _) => match auth::refresh(&cfg.api_key, rt) {
            Ok(s) => s,
            Err(e) if cfg.password.is_some() => {
                log::warn!("refresh failed ({e:#}); signing in with password");
                auth::sign_in_with_password(&cfg.api_key, &cfg.email, cfg.password.as_deref().unwrap())?
            }
            Err(e) => return Err(e).context("refresh token rejected; run `claude-usage-collector login`"),
        },
        (None, Some(pw)) => auth::sign_in_with_password(&cfg.api_key, &cfg.email, pw)?,
        (None, None) => bail!("not signed in: run `claude-usage-collector login` (or set password in config)"),
    };
    let changed = state.refresh_token.as_deref() != Some(&s.refresh_token) || state.uid.as_deref() != Some(&s.uid);
    state.uid = Some(s.uid.clone());
    state.refresh_token = Some(s.refresh_token.clone());
    if changed {
        state.save(&paths::state_file())?;
    }
    scrub_password(cfg, config_path)?;
    Ok(s)
}

fn run(cfg: &Config, args: &Args, home: &Path) -> anyhow::Result<()> {
    let host = cfg.host();
    let state_path = paths::state_file();
    let mut state = State::load(&state_path);
    let mut session: Option<auth::Session> = None;
    let mut scanners: HashMap<PathBuf, Scanner> = HashMap::new();
    let mut days = if let Some(n) = args.backfill { n } else { cfg.days }.max(1);
    let mut first = true;

    log::info!("claude-usage-collector {VERSION} starting; host={host} config={}", args.config.display());

    loop {
        let accts = accounts::discover(home, cfg);
        if accts.is_empty() {
            log::warn!("no accounts found");
        }
        match ensure_session(cfg, &args.config, &mut state, session.take()) {
            Ok(s) => session = Some(s),
            Err(e) => {
                log::error!("auth: {e:#}");
                if args.once {
                    return Err(e);
                }
            }
        }
        let mut any_failed = false;
        if let Some(s) = &session {
            let client = firestore::Client { project_id: &cfg.project_id, id_token: &s.id_token };
            let today = Local::now().date_naive();
            let from = today - chrono::Duration::days(i64::from(days) - 1);
            for a in &accts {
                let sc = scanners.entry(a.dir.clone()).or_default();
                let agg = match sc.scan(&a.data_roots(), a.file_ext(), from, a.parse_fn()) {
                    Ok(x) => x,
                    Err(e) => {
                        log::warn!("{}: {e:#}", a.label);
                        continue;
                    }
                };
                let mut pushed = 0;
                let mut failed = false;
                for (date, models) in &agg {
                    if models.is_empty() {
                        continue;
                    }
                    // One retry per document: transient 403/5xx have been observed
                    // right after a rules deploy. Anything else is logged and the
                    // day is re-pushed on the next loop iteration anyway.
                    let mut ok = false;
                    for attempt in 0..2 {
                        match client.put_day(&s.uid, &host, &a.label, a.provider.as_str(), *date, models) {
                            Ok(()) => {
                                ok = true;
                                break;
                            }
                            Err(e) if attempt == 0 => {
                                log::warn!("{} {date}: {e:#} (retrying)", a.label);
                                std::thread::sleep(Duration::from_secs(2));
                            }
                            Err(e) => log::error!("{} {date}: {e:#}", a.label),
                        }
                    }
                    if ok {
                        pushed += 1;
                    } else {
                        failed = true;
                    }
                }
                if let Err(e) = client.put_account_meta(&s.uid, &host, a, sc.files_seen_last_scan, VERSION) {
                    log::error!("{} meta: {e:#}", a.label);
                    failed = true;
                }
                log::info!(
                    "{}: {} days pushed ({} files seen, {} parsed){}",
                    a.label,
                    pushed,
                    sc.files_seen_last_scan,
                    sc.files_parsed_last_scan,
                    if failed { " — with errors" } else { "" }
                );
                if failed {
                    any_failed = true;
                }
            }
        }
        if first {
            first = false;
            days = cfg.days.max(1); // backfill only on the first pass
        }
        if args.once {
            if any_failed {
                bail!("some documents failed to push (see log)");
            }
            break;
        }
        let _ = std::io::stderr().flush();
        std::thread::sleep(Duration::from_secs(cfg.interval_s.max(30)));
    }
    Ok(())
}
