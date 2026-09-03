// Generic per-day token aggregation over a set of data files with a
// (mtime, len) cache so repeated scans only parse files that changed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{Local, NaiveDate, Utc};

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
pub type Days = BTreeMap<NaiveDate, DayAgg>;
pub type ParseFn = fn(&Path) -> anyhow::Result<Days>;

pub fn add_usage(days: &mut Days, date: NaiveDate, model: &str, t: &ModelTotals) {
    days.entry(date).or_default().entry(model.to_string()).or_default().add(t);
}

#[derive(Clone, Debug)]
struct FileEntry {
    mtime: SystemTime,
    len: u64,
    days: Days,
}

#[derive(Default)]
pub struct Scanner {
    cache: HashMap<PathBuf, FileEntry>,
    pub files_parsed_last_scan: usize,
    pub files_seen_last_scan: usize,
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<(PathBuf, fs::Metadata)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = e.metadata() else { continue };
        if md.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some(ext) {
            out.push((p, md));
        }
    }
}

impl Scanner {
    /// Aggregates per local day for every day >= `from` (inclusive) across all
    /// `*.ext` files under `roots` (recursive). Files whose mtime is older than
    /// `from` are skipped entirely. Files that fail to parse are not cached, so
    /// a transient error (locked SQLite db, partial write) is retried next scan.
    pub fn scan(&mut self, roots: &[PathBuf], ext: &str, from: NaiveDate, parse: ParseFn) -> anyhow::Result<Days> {
        let cutoff_sys: SystemTime = from
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .map(|t| t.with_timezone(&Utc).into())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let mut seen = HashSet::new();
        self.files_parsed_last_scan = 0;
        self.files_seen_last_scan = 0;

        let mut files = Vec::new();
        let mut any_root = false;
        for root in roots {
            if root.is_dir() {
                any_root = true;
                walk(root, ext, &mut files);
            }
        }
        if !any_root {
            anyhow::bail!("none of the data dirs exist: {:?}", roots);
        }
        for (fpath, md) in files {
            let Ok(mtime) = md.modified() else { continue };
            if mtime < cutoff_sys {
                continue;
            }
            let len = md.len();
            seen.insert(fpath.clone());
            self.files_seen_last_scan += 1;
            let fresh = self.cache.get(&fpath).map(|e| e.mtime == mtime && e.len == len).unwrap_or(false);
            if !fresh {
                match parse(&fpath) {
                    Ok(days) => {
                        self.cache.insert(fpath.clone(), FileEntry { mtime, len, days });
                    }
                    Err(e) => {
                        log::warn!("{}: {e:#}", fpath.display());
                        self.cache.remove(&fpath);
                    }
                }
                self.files_parsed_last_scan += 1;
            }
        }
        self.cache.retain(|p, _| seen.contains(p));

        let mut out: Days = BTreeMap::new();
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
