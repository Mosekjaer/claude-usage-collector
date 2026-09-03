// Minimal Firestore REST client: PATCH documents with typed field values.

use std::collections::BTreeMap;

use anyhow::{anyhow, Context};
use chrono::{NaiveDate, Utc};
use serde_json::{json, Map, Value};

use crate::accounts::Account;
use crate::stats::ModelTotals;

pub struct Client<'a> {
    pub project_id: &'a str,
    pub id_token: &'a str,
}

fn int(v: impl ToString) -> Value {
    json!({ "integerValue": v.to_string() })
}
fn string(v: &str) -> Value {
    json!({ "stringValue": v })
}
fn map(fields: Map<String, Value>) -> Value {
    json!({ "mapValue": { "fields": fields } })
}

fn totals_value(t: &ModelTotals) -> Value {
    let mut f = Map::new();
    f.insert("input".into(), int(t.input));
    f.insert("output".into(), int(t.output));
    f.insert("cache_read".into(), int(t.cache_read));
    f.insert("cache_write_5m".into(), int(t.cache_write_5m));
    f.insert("cache_write_1h".into(), int(t.cache_write_1h));
    f.insert("replies".into(), int(t.replies));
    map(f)
}

fn err_message(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => anyhow!("HTTP {code}: {}", resp.into_string().unwrap_or_default()),
        other => anyhow!(other),
    }
}

impl<'a> Client<'a> {
    fn doc_url(&self, name: &str, update_mask: &[String]) -> String {
        let mut url = format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{name}",
            self.project_id
        );
        let mut sep = '?';
        for m in update_mask {
            url.push(sep);
            url.push_str("updateMask.fieldPaths=");
            url.push_str(&urlencode(m));
            sep = '&';
        }
        url
    }

    fn patch(&self, name: &str, fields: Map<String, Value>, update_mask: &[String]) -> anyhow::Result<()> {
        let url = self.doc_url(name, update_mask);
        let body = json!({ "fields": fields });
        if let Err(e) = ureq::request("PATCH", &url)
            .set("Authorization", &format!("Bearer {}", self.id_token))
            .send_json(&body)
            .map_err(err_message)
        {
            log::debug!("request body for {name}: {body}");
            return Err(e).with_context(|| format!("PATCH {name}"));
        }
        Ok(())
    }

    /// PATCH users/{uid}/days/{date}_{host}_{account}. Full document replace; idempotent.
    pub fn put_day(
        &self,
        uid: &str,
        host: &str,
        account: &str,
        date: NaiveDate,
        models: &BTreeMap<String, ModelTotals>,
    ) -> anyhow::Result<()> {
        let name = format!("users/{uid}/days/{date}_{host}_{account}");
        let mut m = Map::new();
        for (model, t) in models {
            m.insert(model.clone(), totals_value(t));
        }
        let mut f = Map::new();
        f.insert("date".into(), string(&date.to_string()));
        f.insert("host".into(), string(host));
        f.insert("account".into(), string(account));
        f.insert("updatedAt".into(), json!({ "timestampValue": Utc::now().to_rfc3339() }));
        f.insert("models".into(), map(m));
        self.patch(&name, f, &[])
    }

    /// PATCH users/{uid} with updateMask=accounts.`{host}/{account}` so other
    /// accounts/hosts and manual fields (displayName) are left untouched.
    pub fn put_account_meta(
        &self,
        uid: &str,
        host: &str,
        account: &Account,
        files_parsed: usize,
        version: &str,
    ) -> anyhow::Result<()> {
        let key = format!("{host}/{}", account.label);
        let mut a = Map::new();
        a.insert("label".into(), string(&account.label));
        a.insert("host".into(), string(host));
        if let Some(d) = &account.display {
            a.insert("display".into(), string(d));
        }
        match &account.subscription {
            Some(s) => {
                a.insert("subscription".into(), string(&s.kind));
                a.insert("tier".into(), string(&s.tier));
                match s.usd {
                    Some(u) => a.insert("subscriptionUsd".into(), int(u)),
                    None => a.insert("subscriptionUsd".into(), json!({ "nullValue": null })),
                };
            }
            None => {
                a.insert("subscriptionUsd".into(), json!({ "nullValue": null }));
            }
        }
        a.insert("lastPush".into(), json!({ "timestampValue": Utc::now().to_rfc3339() }));
        a.insert("version".into(), string(version));
        a.insert("filesParsed".into(), int(files_parsed));
        let mut accounts = Map::new();
        accounts.insert(key.clone(), map(a));
        let mut f = Map::new();
        f.insert("accounts".into(), map(accounts));
        self.patch(&format!("users/{uid}"), f, &[format!("accounts.`{key}`")])
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
