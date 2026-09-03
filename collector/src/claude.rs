// Claude Code transcripts: <config-dir>/projects/**/*.jsonl. Port of
// cosmic-ext-applet-claude-usage/src/stats.rs.

use std::collections::HashSet;
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

pub fn parse_file(path: &Path) -> anyhow::Result<Days> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut days = Days::new();
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
        let t = ModelTotals {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_read: usage.cache_read_input_tokens,
            cache_write_5m: w5,
            cache_write_1h: w1,
            replies: 1,
        };
        add_usage(&mut days, date, model, &t);
    }
    Ok(days)
}
