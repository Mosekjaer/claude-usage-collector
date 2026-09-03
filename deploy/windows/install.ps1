# Installs claude-usage-collector.exe and registers a Task Scheduler task
# that starts it at logon (runs as the current user, no admin needed).
# Run from the extracted release zip:  powershell -ExecutionPolicy Bypass -File install.ps1
$ErrorActionPreference = 'Stop'
$dst = Join-Path $env:LOCALAPPDATA 'Programs\claude-usage-collector'
New-Item -ItemType Directory -Force $dst | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'claude-usage-collector.exe') $dst -Force
$exe = Join-Path $dst 'claude-usage-collector.exe'

$cfg = Join-Path $env:APPDATA 'claude-usage-collector\config.toml'
if (-not (Test-Path $cfg)) {
  & $exe init
  Write-Host ""
  Write-Host "1. Edit $cfg  (set email)"
  Write-Host "2. Run:  `"$exe`" login"
  Write-Host "3. Run:  `"$exe`" accounts   (check that your Claude dirs are found)"
  Write-Host "4. Re-run this script to register the logon task."
  exit 0
}

schtasks /create /f /sc onlogon /rl limited /tn "ClaudeUsageCollector" /tr "`"$exe`"" | Out-Null
schtasks /run /tn "ClaudeUsageCollector" | Out-Null
Write-Host "Installed. Task 'ClaudeUsageCollector' runs at logon. Log: $env:APPDATA\claude-usage-collector\collector.log"
