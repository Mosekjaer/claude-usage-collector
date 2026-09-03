// Codex CLI rollouts: <config-dir>/sessions/YYYY/MM/DD/rollout-*.jsonl.
// Usage comes from `event_msg` lines with payload.type == "token_count":
// `info.total_token_usage` is cumulative for the session, `info.last_token_usage`
// is the last API call. We use deltas of the cumulative counters so a repeated
// event never double-counts. The model comes from the preceding `turn_context`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::{DateTime, Local};
use serde::Deserialize;

use crate::stats::{add_usage, Days, ModelTotals};

#[derive(Deserialize)]
struct Line<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    cache_write_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

fn sub(a: &Usage, b: &Usage) -> Option<Usage> {
    Some(Usage {
        input_tokens: a.input_tokens.checked_sub(b.input_tokens)?,
        cached_input_tokens: a.cached_input_tokens.checked_sub(b.cached_input_tokens)?,
        cache_write_input_tokens: a.cache_write_input_tokens.checked_sub(b.cache_write_input_tokens)?,
        output_tokens: a.output_tokens.checked_sub(b.output_tokens)?,
    })
}

pub fn parse_file(path: &Path) -> anyhow::Result<Days> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut days = Days::new();
    let mut model = String::from("codex-unknown");
    let mut prev_total: Option<Usage> = None;
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if !(line.contains("\"token_count\"") || line.contains("\"turn_context\"")) {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Line>(&line) else { continue };
        let Some(payload) = parsed.payload else { continue };
        match parsed.kind {
            "turn_context" => {
                if let Some(m) = payload.get("model").and_then(|m| m.as_str()) {
                    model = m.to_string();
                }
            }
            "event_msg" if payload.get("type").and_then(|t| t.as_str()) == Some("token_count") => {
                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else { continue };
                let total: Usage = info.get("total_token_usage").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                let last: Usage = info.get("last_token_usage").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                // Delta of cumulative counters; fall back to `last` if the
                // cumulative counter went backwards (new baseline).
                let delta = match prev_total {
                    Some(p) => sub(&total, &p).unwrap_or(last),
                    None => total,
                };
                prev_total = Some(total);
                if delta == Usage::default() {
                    continue;
                }
                let Some(ts) = parsed.timestamp.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { continue };
                let date = ts.with_timezone(&Local).date_naive();
                let t = ModelTotals {
                    input: delta.input_tokens.saturating_sub(delta.cached_input_tokens),
                    output: delta.output_tokens,
                    cache_read: delta.cached_input_tokens,
                    cache_write_5m: delta.cache_write_input_tokens,
                    cache_write_1h: 0,
                    replies: 1,
                };
                add_usage(&mut days, date, &model, &t);
            }
            _ => {}
        }
    }
    Ok(days)
}

/// `rate_limits.plan_type` from the most recently modified rollout, e.g.
/// "plus", "pro", "free", "team".
pub fn read_plan_type(sessions_dir: &Path) -> Option<String> {
    fn walk(dir: &Path, out: &mut Vec<(std::path::PathBuf, fs::Metadata)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                out.push((p, md));
            }
        }
    }
    let mut files = Vec::new();
    walk(sessions_dir, &mut files);
    files.sort_by_key(|(_, m)| std::cmp::Reverse(m.modified().ok()));
    for (p, _) in files.into_iter().take(5) {
        let Ok(f) = fs::File::open(&p) else { continue };
        let mut plan = None;
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if let Some(i) = line.find("\"plan_type\":\"") {
                let rest = &line[i + 13..];
                if let Some(end) = rest.find('"') {
                    plan = Some(rest[..end].to_string());
                }
            }
        }
        if plan.is_some() {
            return plan;
        }
    }
    None
}

/// ChatGPT Plus $20, Pro $200, free $0. Team/business/enterprise: unknown.
pub fn plan_usd(plan: &str) -> Option<u32> {
    match plan {
        "plus" => Some(20),
        "pro" => Some(200),
        "free" => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn deltas_dedup_and_model_switch() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rollout.jsonl");
        fs::write(&p, include_str!("../tests/fixtures/codex_sample.jsonl")).unwrap();
        let days = parse_file(&p).unwrap();
        // Local dates: fixture times are mid-day UTC, so the local date matches.
        let d1 = &days[&NaiveDate::from_ymd_opt(2026, 9, 2).unwrap()]["gpt-5.6-sol"];
        assert_eq!(d1.replies, 2, "repeated cumulative counter is not counted twice");
        assert_eq!(d1.input, (1000 - 600) + (2000 - 1500));
        assert_eq!(d1.cache_read, 600 + 1500);
        assert_eq!(d1.output, 100 + 250);
        // Counter reset (3000 -> 500) falls back to last_token_usage.
        let d2 = &days[&NaiveDate::from_ymd_opt(2026, 9, 3).unwrap()]["gpt-5.5"];
        assert_eq!((d2.input, d2.output, d2.replies), (500, 50, 1));

        fs::create_dir_all(tmp.path().join("sessions/2026/09/03")).unwrap();
        fs::copy(&p, tmp.path().join("sessions/2026/09/03/rollout.jsonl")).unwrap();
        assert_eq!(read_plan_type(&tmp.path().join("sessions")).as_deref(), Some("pro"));
        assert_eq!(plan_usd("plus"), Some(20));
    }
}
