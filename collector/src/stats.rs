// Token statistics from Claude Code transcripts in <config-dir>/projects/*/*.jsonl.
// Port of cosmic-ext-applet-claude-usage/src/stats.rs, reduced to a per-day
// aggregate. Per-file results are cached on (mtime, len) so repeated scans only
// parse files that changed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub replies: u32,
}

impl ModelTotals {
    pub fn add(&mut self, o: &ModelTotals) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write_5m += o.cache_write_5m;
        self.cache_write_1h += o.cache_write_1h;
        self.replies += o.replies;
    }
}

/// model id -> totals
pub type DayAgg = BTreeMap<String, ModelTotals>;

#[derive(Clone, Debug)]
struct FileEntry {
    mtime: SystemTime,
    len: u64,
    days: BTreeMap<NaiveDate, DayAgg>,
}

#[derive(Default)]
pub struct Scanner {
    cache: HashMap<PathBuf, FileEntry>,
    pub files_parsed_last_scan: usize,
    pub files_seen_last_scan: usize,
}

impl Scanner {
    /// Aggregates per local day for every day >= `from` (inclusive). Files whose
    /// mtime is older than `from` are skipped entirely.
    pub fn scan(&mut self, root: &Path, from: NaiveDate) -> anyhow::Result<BTreeMap<NaiveDate, DayAgg>> {
        let cutoff_local = from.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).earliest();
        let cutoff_sys: SystemTime = match cutoff_local {
            Some(t) => t.with_timezone(&Utc).into(),
            None => SystemTime::UNIX_EPOCH,
        };

        let mut seen = HashSet::new();
        self.files_parsed_last_scan = 0;
        self.files_seen_last_scan = 0;

        let projects = match fs::read_dir(root) {
            Ok(rd) => rd,
            Err(e) => anyhow::bail!("cannot read {}: {e}", root.display()),
        };
        for project in projects.flatten() {
            let ppath = project.path();
            if !ppath.is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(&ppath) else { continue };
            for f in files.flatten() {
                let fpath = f.path();
                if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(md) = f.metadata() else { continue };
                let Ok(mtime) = md.modified() else { continue };
                if mtime < cutoff_sys {
                    continue;
                }
                let len = md.len();
                seen.insert(fpath.clone());
                self.files_seen_last_scan += 1;
                let fresh = self.cache.get(&fpath).map(|e| e.mtime == mtime && e.len == len).unwrap_or(false);
                if !fresh {
                    let days = parse_file(&fpath).unwrap_or_default();
                    self.cache.insert(fpath.clone(), FileEntry { mtime, len, days });
                    self.files_parsed_last_scan += 1;
                }
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

        let mut out: BTreeMap<NaiveDate, DayAgg> = BTreeMap::new();
        for entry in self.cache.values() {
            for (date, day) in &entry.days {
                if *date < from {
                    continue;
                }
                let agg = out.entry(*date).or_default();
                for (model, t) in day {
                    agg.entry(model.clone()).or_default().add(t);
                }
            }
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct Line<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(rename = "requestId", default)]
    request_id: Option<&'a str>,
    #[serde(default)]
    message: Option<Msg<'a>>,
}

#[derive(Deserialize)]
struct Msg<'a> {
    #[serde(default)]
    id: Option<&'a str>,
    #[serde(default)]
    model: Option<&'a str>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize, Default)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

pub fn parse_file(path: &Path) -> anyhow::Result<BTreeMap<NaiveDate, DayAgg>> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut days: BTreeMap<NaiveDate, DayAgg> = BTreeMap::new();
    let mut dedup: HashSet<(String, String)> = HashSet::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Line>(&line) else { continue };
        if parsed.kind != "assistant" {
            continue;
        }
        let Some(msg) = parsed.message else { continue };
        let Some(usage) = msg.usage else { continue };
        let model = msg.model.unwrap_or("unknown");
        // Only Claude models: local/proxied models (Ollama etc.) are skipped.
        if !model.starts_with("claude-") {
            continue;
        }
        let key = (msg.id.unwrap_or("").to_string(), parsed.request_id.unwrap_or("").to_string());
        if !key.0.is_empty() && !dedup.insert(key) {
            continue;
        }
        let Some(ts) = parsed.timestamp.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { continue };
        let date = ts.with_timezone(&Local).date_naive();
        let (w5, w1) = match &usage.cache_creation {
            Some(c) if c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens > 0 => {
                (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens)
            }
            _ => (usage.cache_creation_input_tokens, 0),
        };
        let t = days.entry(date).or_default().entry(model.to_string()).or_default();
        t.input += usage.input_tokens;
        t.output += usage.output_tokens;
        t.cache_read += usage.cache_read_input_tokens;
        t.cache_write_5m += w5;
        t.cache_write_1h += w1;
        t.replies += 1;
    }
    Ok(days)
}
