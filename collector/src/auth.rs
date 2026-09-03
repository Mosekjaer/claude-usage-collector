// Firebase Auth via the Identity Toolkit REST API. No SDK.

use anyhow::{anyhow, Context};
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Session {
    pub uid: String,
    pub id_token: String,
    pub refresh_token: String,
    pub expires_at: Instant,
}

impl Session {
    pub fn needs_refresh(&self) -> bool {
        Instant::now() + Duration::from_secs(300) >= self.expires_at
    }
}

#[derive(Deserialize)]
struct SignInResp {
    #[serde(rename = "idToken")]
    id_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "localId")]
    local_id: String,
    #[serde(rename = "expiresIn")]
    expires_in: String,
}

#[derive(Deserialize)]
struct RefreshResp {
    id_token: String,
    refresh_token: String,
    user_id: String,
    expires_in: String,
}

#[derive(Deserialize)]
struct ErrResp {
    error: ErrBody,
}
#[derive(Deserialize)]
struct ErrBody {
    message: String,
}

fn err_message(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<ErrResp>(&body).map(|e| e.error.message).unwrap_or(body);
            anyhow!("HTTP {code}: {msg}")
        }
        other => anyhow!(other),
    }
}

pub fn sign_in_with_password(api_key: &str, email: &str, password: &str) -> anyhow::Result<Session> {
    let url = format!("https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key={api_key}");
    let resp = ureq::post(&url)
        .send_json(json!({ "email": email, "password": password, "returnSecureToken": true }))
        .map_err(err_message)
        .context("signInWithPassword")?;
    let r: SignInResp = resp.into_json()?;
    let secs: u64 = r.expires_in.parse().unwrap_or(3600);
    Ok(Session {
        uid: r.local_id,
        id_token: r.id_token,
        refresh_token: r.refresh_token,
        expires_at: Instant::now() + Duration::from_secs(secs),
    })
}

pub fn refresh(api_key: &str, refresh_token: &str) -> anyhow::Result<Session> {
    let url = format!("https://securetoken.googleapis.com/v1/token?key={api_key}");
    let resp = ureq::post(&url)
        .send_form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token)])
        .map_err(err_message)
        .context("token refresh")?;
    let r: RefreshResp = resp.into_json()?;
    let secs: u64 = r.expires_in.parse().unwrap_or(3600);
    Ok(Session {
        uid: r.user_id,
        id_token: r.id_token,
        refresh_token: r.refresh_token,
        expires_at: Instant::now() + Duration::from_secs(secs),
    })
}
