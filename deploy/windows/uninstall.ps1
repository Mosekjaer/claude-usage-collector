$ErrorActionPreference = 'SilentlyContinue'
schtasks /end /tn "ClaudeUsageCollector" | Out-Null
schtasks /delete /f /tn "ClaudeUsageCollector" | Out-Null
Stop-Process -Name claude-usage-collector -Force
Remove-Item -Recurse -Force (Join-Path $env:LOCALAPPDATA 'Programs\claude-usage-collector')
Write-Host "Removed. Config left in $env:APPDATA\claude-usage-collector (delete manually if wanted)."
