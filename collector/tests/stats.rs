use std::fs;
use std::path::PathBuf;

use chrono::Local;

#[path = "../src/stats.rs"]
mod stats;
#[path = "../src/claude.rs"]
mod claude;

fn fixture_root() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("-home-frederik-proj");
    fs::create_dir_all(&proj).unwrap();
    let src = include_str!("fixtures/transcript_sample.jsonl");
    let today = Local::now().date_naive();
    let old = today - chrono::Duration::days(8);
    let src = src
        .replace("2026-09-02", &today.format("%Y-%m-%d").to_string())
        .replace("2026-08-25", &old.format("%Y-%m-%d").to_string());
    fs::write(proj.join("session-a.jsonl"), src).unwrap();
    let root = tmp.path().to_path_buf();
    (tmp, root)
}

#[test]
fn dedups_buckets_and_caches() {
    let (_tmp, root) = fixture_root();
    let today = Local::now().date_naive();
    let old = today - chrono::Duration::days(8);
    let mut sc = stats::Scanner::default();

    // Window covering only the last 3 days: the 8-day-old line is excluded.
    let agg = sc.scan(&[root.clone()], "jsonl", today - chrono::Duration::days(2), claude::parse_file).unwrap();
    assert_eq!(sc.files_parsed_last_scan, 1);
    let t = &agg[&today];
    let opus = &t["claude-opus-5"];
    assert_eq!(opus.replies, 2, "duplicate (msg id, request id) counts once");
    assert_eq!(opus.input, 2 + 5);
    assert_eq!(opus.cache_write_1h, 18228);
    assert_eq!(opus.cache_write_5m, 100);
    let fable = &t["claude-fable-5-1"];
    assert_eq!(fable.cache_write_5m, 500, "no breakdown → 5 min bucket");
    assert_eq!(fable.cache_write_1h, 0);
    assert!(t.contains_key("claude-newmodel-9"));
    assert!(!t.contains_key("huihui_ai/qwen3"));
    assert!(!agg.contains_key(&old));

    // 30-day window includes the old day.
    let agg = sc.scan(&[root.clone()], "jsonl", today - chrono::Duration::days(29), claude::parse_file).unwrap();
    assert_eq!(sc.files_parsed_last_scan, 0, "unchanged file is not re-parsed");
    assert_eq!(agg[&old]["claude-haiku-4-5"].input, 1000);
}
