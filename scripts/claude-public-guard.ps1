# Claude Code PreToolUse hook: runs the public-safety check before git commit/push,
# even in clones where core.hooksPath has not been configured.
$ErrorActionPreference = 'Stop'
$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
$command = [string]$payload.tool_input.command
if ($command -notmatch '\bgit\b[^|;&]*\b(commit|push)\b') { exit 0 }
$mode = if ($command -match '\bpush\b') { '-Push' } else { '-Staged' }
if ($command -match '--no-verify') {
    [Console]::Error.WriteLine('This is a public repository: do not bypass the public-safety hooks with --no-verify.')
    exit 2
}
$output = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'check-public.ps1') $mode 2>&1
if ($LASTEXITCODE -ne 0) {
    [Console]::Error.WriteLine(($output | Out-String))
    exit 2
}
exit 0
