# claude-usage-collector

Shared dashboard for a small team on AI coding subscriptions. A tiny daemon on
each machine (Linux, Windows) reads the local session logs of Claude Code,
Codex CLI and Antigravity, pushes raw per-day / per-model token counts to
Firestore, and a static web app prices them at API rates against what the
subscriptions actually cost.

| Provider | Data read | Subscription |
|---|---|---|
| Claude Code | `~/.claude*/projects/**/*.jsonl` (`assistant` messages with `usage`) | derived from `.credentials.json` (Pro $20, Max 5× $100, Max 20× $200) |
| Codex CLI | `~/.codex/sessions/**/*.jsonl` (`token_count` events, deltas of the cumulative counter) | derived from `rate_limits.plan_type` (plus $20, pro $200, free $0) |
| Antigravity IDE + CLI | `~/.gemini/antigravity*/conversations/*.db` (SQLite, `gen_metadata` protobuf blobs) | not on disk; set `subscription_usd` |

Antigravity has no documented usage log. The `gen_metadata` blob layout was
reverse-engineered (see `collector/src/antigravity.rs`): field 1.4 holds
uncached input (2), cached input (5) and output (3 = reasoning 9 + text 10)
token counts, 1.21 the model display name, 1.9.4.1 the timestamp. Verified on
534 generations; treat the numbers as best-effort.

Gemini CLI is not supported: personal OAuth was retired in favour of
Antigravity, and its session files carry no token counts.

Dashboard: https://claude-usage-collector-fm.web.app

Only numbers leave the machine: token counts, model ids, the subscription type
from `.credentials.json`, and the hostname. Never transcript content, never
OAuth tokens.

## Install

### Linux

```
tar xzf claude-usage-collector-linux-x86_64.tar.gz
cd claude-usage-collector
./install.sh ./claude-usage-collector     # copies to ~/.local/bin, writes config template
$EDITOR ~/.config/claude-usage-collector/config.toml    # set email
claude-usage-collector login              # prompts for password, stores a refresh token
claude-usage-collector accounts           # check the Claude dirs it found
claude-usage-collector --once --backfill 90
systemctl --user enable --now claude-usage-collector
```

From source: `deploy/linux/install.sh` with no argument runs `cargo build --release` first.

### Windows

Download `claude-usage-collector-windows-x86_64.zip` from the latest release, extract, then in PowerShell:

```
powershell -ExecutionPolicy Bypass -File install.ps1    # writes config template
notepad $env:APPDATA\claude-usage-collector\config.toml # set email
& "$env:LOCALAPPDATA\Programs\claude-usage-collector\claude-usage-collector.exe" login
& "$env:LOCALAPPDATA\Programs\claude-usage-collector\claude-usage-collector.exe" accounts
& "$env:LOCALAPPDATA\Programs\claude-usage-collector\claude-usage-collector.exe" --once --backfill 90
powershell -ExecutionPolicy Bypass -File install.ps1    # now registers the logon task
```

The release build has no console window; `accounts`, `login` and `paths`
attach to the terminal you run them from.

## Config

`claude-usage-collector paths` prints where config, state and log live.
`claude-usage-collector init` writes a commented template:

```toml
api_key    = "AIza..."         # Firebase web API key (public)
project_id = "claude-usage-collector-fm"
email      = "you@example.com"
interval_s = 300
days       = 3                 # local days re-pushed every run
auto_discover = true           # ~/.claude* with projects/, plus $CLAUDE_CONFIG_DIR

[[accounts]]                   # any number, anywhere on disk
path    = "~/.claude"
display = "Max 20x"

[[accounts]]
path    = "~/.claude-5x"
display = "Max 5x"
```

Each account is one provider config dir (`provider` is detected from the
layout: `projects/` → claude, `sessions/` → codex, `antigravity*/conversations/`
→ antigravity; set it explicitly for other paths). Subscription prices are
derived where possible (see table above); unknown ones show as `?`. Set
`subscription_usd` on the account to override, including `0` when someone
else pays.

```toml
[[accounts]]
path = "~/.codex"
display = "ChatGPT Plus"

[[accounts]]
path = "~/.gemini"
provider = "antigravity"
display = "Google AI Pro"
subscription_usd = 0
```

`claude-usage-collector scan --backfill 30` prints per-model totals per
account without pushing anything.

## Data model

`users/{uid}/days/{YYYY-MM-DD}_{host}_{account}`:

```json
{ "date": "2026-09-03", "host": "laptop", "account": "claude", "provider": "claude", "updatedAt": "...",
  "models": { "claude-fable-5-1": { "input": 1312, "output": 48210, "cache_read": 9812331,
                                     "cache_write_5m": 0, "cache_write_1h": 401220, "replies": 143 } } }
```

`users/{uid}.accounts["host/label"]` carries display name, subscription, tier,
`subscriptionUsd`, `lastPush`. `users/{uid}.displayName` is set by hand in the
Firebase console.

Rules (`firestore.rules`): any signed-in user reads everything, each user
writes only under their own uid. Sign-up is disabled; accounts are created in
the console.

## Web app

Static files in `web/`, hosted on Firebase Hosting. Prices live in
`web/pricing.js` (Anthropic, OpenAI GPT-5.5/5.6, Gemini 3.x Pro; longest
model-id prefix wins); update there and `firebase deploy --only hosting`.

The Firebase web API key in `web/app.js` is a public client identifier, not a
secret: it is restricted to Identity Toolkit, Secure Token and Firestore, and
access is governed by Firestore rules plus disabled sign-up.

## Development

```
cargo test
cargo run -- accounts
cargo run -- --once --debug
firebase deploy --only firestore:rules,hosting
```

Releases: push a `v*` tag; `.github/workflows/release.yml` builds Linux and
Windows binaries and attaches them to the GitHub release.
