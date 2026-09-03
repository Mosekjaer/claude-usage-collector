#!/usr/bin/env bash
# Installs the collector as a systemd --user service.
# Usage: deploy/linux/install.sh [path/to/claude-usage-collector]
# Without an argument it builds from source with cargo.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="${1:-}"
if [[ -z "$bin" ]]; then
  (cd "$repo" && cargo build --release)
  bin="$repo/target/release/claude-usage-collector"
fi
install -Dm755 "$bin" "$HOME/.local/bin/claude-usage-collector"
install -Dm644 "$here/claude-usage-collector.service" "$HOME/.config/systemd/user/claude-usage-collector.service"
cfg="$HOME/.config/claude-usage-collector/config.toml"
if [[ ! -f "$cfg" ]]; then
  "$HOME/.local/bin/claude-usage-collector" init
  echo "edit $cfg (email), then run: claude-usage-collector login"
  echo "then: systemctl --user enable --now claude-usage-collector"
  exit 0
fi
systemctl --user daemon-reload
systemctl --user enable --now claude-usage-collector
systemctl --user --no-pager status claude-usage-collector | head -5
