// Antigravity (IDE and CLI) conversations: <dir>/antigravity*/conversations/*.db.
// Each SQLite db has a `gen_metadata` table with one protobuf blob per model
// generation. Reverse-engineered layout (verified on 534 rows, 2026-09):
//
//   1 (message)
//     4 (usage)   2 = uncached input tokens, 5 = cached input tokens,
//                 3 = output tokens (= 9 reasoning + 10 text)
//     9.4.1       unix timestamp (seconds)
//     19          model id-ish string, e.g. "gemini-pro-default", "claude-opus-4-6-thinking"
//     21          model display name, e.g. "Gemini 3.1 Pro (High)"
//
// Model ids are normalised from the display name: lowercase, parenthetical
// dropped, spaces and dots to '-': "Gemini 3.1 Pro (High)" -> "gemini-3-1-pro",
// "Claude Opus 4.6 (Thinking)" -> "claude-opus-4-6".

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use rusqlite::{Connection, OpenFlags};

use crate::proto;
use crate::stats::{add_usage, Days, ModelTotals};

pub fn data_roots(dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        dir.join("antigravity").join("conversations"),
        dir.join("antigravity-cli").join("conversations"),
    ];
    if dir.join("conversations").is_dir() {
        v.push(dir.join("conversations"));
    }
    v
}

pub fn normalize_model(display: &str) -> String {
    let base = display.split('(').next().unwrap_or(display).trim();
    let mut s = String::new();
    let mut last_dash = true;
    for c in base.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            s.push(c);
            last_dash = false;
        } else if !last_dash {
            s.push('-');
            last_dash = true;
        }
    }
    s.trim_end_matches('-').to_string()
}

pub struct Gen {
    pub ts: i64,
    pub model: String,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

pub fn decode_gen(blob: &[u8]) -> Option<Gen> {
    let top = proto::fields(blob)?;
    let m = proto::sub(&top, 1)?;
    let usage = proto::sub(&m, 4)?;
    let ts = proto::sub(&m, 9).and_then(|f| proto::sub(&f, 4)).and_then(|f| proto::int(&f, 1))? as i64;
    let display = proto::str(&m, 21).or_else(|| proto::str(&m, 19)).unwrap_or("antigravity-unknown");
    Some(Gen {
        ts,
        model: normalize_model(display),
        input: proto::int(&usage, 2).unwrap_or(0),
        cached: proto::int(&usage, 5).unwrap_or(0),
        output: proto::int(&usage, 3).unwrap_or(0),
    })
}

pub fn parse_db(path: &Path) -> anyhow::Result<Days> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut days = Days::new();
    let mut stmt = conn.prepare("SELECT data FROM gen_metadata")?;
    let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
    for blob in rows.flatten() {
        let Some(g) = decode_gen(&blob) else { continue };
        let Some(dt) = DateTime::from_timestamp(g.ts, 0) else { continue };
        let date = dt.with_timezone(&Local).date_naive();
        let t = ModelTotals {
            input: g.input,
            output: g.output,
            cache_read: g.cached,
            cache_write_5m: 0,
            cache_write_1h: 0,
            replies: 1,
        };
        add_usage(&mut days, date, &g.model, &t);
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_model_names() {
        assert_eq!(normalize_model("Gemini 3.1 Pro (High)"), "gemini-3-1-pro");
        assert_eq!(normalize_model("Claude Opus 4.6 (Thinking)"), "claude-opus-4-6");
        assert_eq!(normalize_model("GPT-OSS 120B"), "gpt-oss-120b");
    }

    #[test]
    fn decodes_real_blob() {
        let hex = include_str!("../tests/fixtures/antigravity_gen_metadata.hex").trim();
        let blob: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let g = decode_gen(&blob).expect("decodes");
        assert_eq!(g.model, "gemini-3-1-pro");
        assert_eq!((g.input, g.cached, g.output), (9920, 8112, 538));
        assert_eq!(g.ts, 1781301747);
    }

    #[test]
    fn parses_sqlite_db() {
        let hex = include_str!("../tests/fixtures/antigravity_gen_metadata.hex").trim();
        let blob: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x.db");
        let c = Connection::open(&p).unwrap();
        c.execute("CREATE TABLE gen_metadata (idx integer, data blob, size integer, PRIMARY KEY (idx))", []).unwrap();
        c.execute("INSERT INTO gen_metadata VALUES (0, ?1, ?2)", rusqlite::params![blob, blob.len() as i64]).unwrap();
        c.execute("INSERT INTO gen_metadata VALUES (1, ?1, ?2)", rusqlite::params![blob, blob.len() as i64]).unwrap();
        c.execute("INSERT INTO gen_metadata VALUES (2, X'00ff', 2)", []).unwrap();
        drop(c);
        let days = parse_db(&p).unwrap();
        let (_, agg) = days.iter().next().unwrap();
        let t = &agg["gemini-3-1-pro"];
        assert_eq!(t.replies, 2);
        assert_eq!(t.input, 2 * 9920);
        assert_eq!(t.cache_read, 2 * 8112);
        assert_eq!(t.output, 2 * 538);
    }
}
