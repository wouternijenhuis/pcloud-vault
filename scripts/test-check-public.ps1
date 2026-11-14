# Self-test: the public-safety check must pass on the repository and fail on planted secrets.
$ErrorActionPreference = 'Stop'
$check = Join-Path $PSScriptRoot 'check-public.ps1'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ("check-public-" + [Guid]::NewGuid() + '.txt')
$fakeToken = 'gh' + 'o_' + ('A1b2C3d4E5' * 4)
$fakePath = 'C:' + '\Users\' + 'someone\secret.txt'
$fakeEmail = 'person' + '@' + 'gmail.com'
try {
    foreach ($sample in @($fakeToken, $fakePath, $fakeEmail)) {
        Set-Content -LiteralPath $fixture -Value "value = $sample"
        & pwsh -NoProfile -File $check -Path $fixture *> $null
        if ($LASTEXITCODE -eq 0) { throw "check-public.ps1 did not flag a planted sample" }
    }
    Set-Content -LiteralPath $fixture -Value 'contact = alex@example.com; path = D:\EncryptedVault'
    & pwsh -NoProfile -File $check -Path $fixture *> $null
    if ($LASTEXITCODE -ne 0) { throw 'check-public.ps1 flagged allowed placeholder data' }
} finally { Remove-Item -LiteralPath $fixture -ErrorAction SilentlyContinue }
Write-Host 'check-public self-test passed.'
