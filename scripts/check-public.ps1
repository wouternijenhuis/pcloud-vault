<#
.SYNOPSIS
  Blocks personal data, local paths, and secrets from entering this public repository.
.DESCRIPTION
  Default: scans all tracked files. -Staged: scans the index (pre-commit) and the author identity.
  -Push: scans tracked files plus author/committer emails of commits not yet on any remote.
  -Path: scans the given files (used by the self-test).
  Extra personal terms can be listed, one per line, in the untracked file .git/info/public-denylist.
#>
[CmdletBinding()]
param(
    [switch]$Staged,
    [switch]$Push,
    [string[]]$Path
)
$ErrorActionPreference = 'Stop'
Set-Location (git rev-parse --show-toplevel)

$noreplyPattern = '^[A-Za-z0-9-]+(\+[A-Za-z0-9-]+)?@users\.noreply\.github\.com$|^noreply@(anthropic\.com|github\.com)$'
$allowedEmailDomains = '(example\.(com|org|net)|[a-z0-9.-]+\.(invalid|test|example)|users\.noreply\.github\.com|anthropic\.com)$'
$skipFiles = '^(Cargo\.lock|pnpm-lock\.yaml|frontend/pnpm-lock\.yaml|LICENSE|scripts/check-public\.ps1)$'
$binaryExtensions = '\.(png|ico|icns|jpg|jpeg|gif|webp|woff2?|ttf|exe|dll|pdf|zip)$'
$forbiddenFiles = '(^|/)\.env(\..*)?$|\.pcloud-recovery$|\.log$|(^|/)\.pcloud-vault/|(^|/)objects/[cmh]-[^/]+\.obj$|(^|/)(qa-state|qa-fixture)/'
$forbiddenFileAllow = '(^|/)\.env\.example$'

$contentRules = [ordered]@{
    'local Windows user path' = '[A-Za-z]:[\\/]+Users[\\/]+[^\\/\s''"`<>]+'
    'local Unix home path'    = '(?<![A-Za-z0-9_.-])/(Users|home)/[a-z][a-z0-9_-]+'
    'GitHub token'            = '\b(gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b'
    'API key'                 = '\b(sk-[A-Za-z0-9_-]{24,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{10,})\b'
    'private key block'       = '-----BEGIN ([A-Z]+ )?PRIVATE KEY-----'
    'bearer/access token'     = '(access_token|auth|bearer)["'':= ]+[A-Za-z0-9._~+/-]{32,}'
}

$denyTerms = @()
$denyFile = Join-Path (git rev-parse --git-dir) 'info/public-denylist'
if (Test-Path -LiteralPath $denyFile) {
    $denyTerms = Get-Content -LiteralPath $denyFile | ForEach-Object { $_.Trim() } | Where-Object { $_ -and -not $_.StartsWith('#') }
}

$findings = [System.Collections.Generic.List[string]]::new()

function Test-Content([string]$name, [string]$text) {
    $lineNumber = 0
    foreach ($line in $text -split "`n") {
        $lineNumber++
        foreach ($rule in $contentRules.GetEnumerator()) {
            if ($line -match $rule.Value) { $findings.Add("${name}:${lineNumber}: $($rule.Key)") }
        }
        foreach ($m in [regex]::Matches($line, '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}')) {
            $domain = $m.Value.Split('@')[1].ToLowerInvariant()
            if ($domain -notmatch $allowedEmailDomains) { $findings.Add("${name}:${lineNumber}: email address outside example/test domains") }
        }
        foreach ($term in $denyTerms) {
            if ($line.IndexOf($term, [StringComparison]::OrdinalIgnoreCase) -ge 0) { $findings.Add("${name}:${lineNumber}: term from local denylist") }
        }
    }
}

function Test-File([string]$name, [scriptblock]$read) {
    if ($name -match $forbiddenFiles -and $name -notmatch $forbiddenFileAllow) { $findings.Add("${name}: forbidden file type for a public repository") }
    if ($name -match $skipFiles -or $name -match $binaryExtensions) { return }
    Test-Content $name (& $read)
}

function Test-Identity([string]$label, [string]$email) {
    if ($email -notmatch $noreplyPattern) { $findings.Add("${label}: '$email' is not a GitHub noreply address") }
}

if ($Path) {
    foreach ($p in $Path) { Test-File ($p -replace '\\', '/') { Get-Content -LiteralPath $p -Raw } }
} elseif ($Staged) {
    $names = git diff --cached --name-only --diff-filter=ACMR
    foreach ($n in $names) { Test-File $n { (git show ":$n") -join "`n" } }
    $ident = git var GIT_AUTHOR_IDENT
    if ($ident -match '<([^>]+)>') { Test-Identity 'commit author' $Matches[1] }
} else {
    foreach ($n in (git ls-files)) {
        if (Test-Path -LiteralPath $n -PathType Leaf) { Test-File $n { Get-Content -LiteralPath $n -Raw } }
    }
    if ($Push) {
        foreach ($line in (git log --format='%h %ae %ce' --branches --not --remotes)) {
            $parts = $line -split ' '
            Test-Identity "commit $($parts[0]) author" $parts[1]
            Test-Identity "commit $($parts[0]) committer" $parts[2]
        }
    }
}

if ($findings.Count -gt 0) {
    Write-Host 'Public-safety check failed. This repository is public; remove the data below (do not weaken the check):' -ForegroundColor Red
    $findings | Sort-Object -Unique | ForEach-Object { Write-Host "  $_" }
    exit 1
}
Write-Host 'Public-safety check passed.'
