# claude-usage-collector

Shared dashboard for a small team on Claude Max subscriptions. A tiny daemon on
each machine (Linux, Windows) reads the Claude Code transcripts in every Claude
config dir it can find, pushes raw per-day / per-model token counts to
Firestore, and a static web app prices them at API rates against what the
subscriptions actually cost.

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

Each account is one Claude config dir. The subscription price is derived from
`<dir>/.credentials.json` (`subscriptionType` + `rateLimitTier`: Pro $20,
Max 5× $100, Max 20× $200). Unknown tiers show as `?`; set `subscription_usd`
on the account to override.

## Data model

`users/{uid}/days/{YYYY-MM-DD}_{host}_{account}`:

```json
{ "date": "2026-09-03", "host": "laptop", "account": "claude", "updatedAt": "...",
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
`web/pricing.js` (mirror of the COSMIC applet's `pricing.rs`); update there
and `firebase deploy --only hosting`.

## Development

```
cargo test
cargo run -- accounts
cargo run -- --once --debug
firebase deploy --only firestore:rules,hosting
```

Releases: push a `v*` tag; `.github/workflows/release.yml` builds Linux and
Windows binaries and attaches them to the GitHub release.
