$ErrorActionPreference = 'Stop'
$taskToolchainRoot = if ($env:PCLOUD_VAULT_TOOLCHAINS) { [IO.Path]::GetFullPath($env:PCLOUD_VAULT_TOOLCHAINS) } else { [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\toolchains')) }
if (Test-Path -LiteralPath (Join-Path $taskToolchainRoot 'cargo\bin\cargo.exe')) {
    $env:CARGO_HOME = Join-Path $taskToolchainRoot 'cargo'
    $env:RUSTUP_HOME = Join-Path $taskToolchainRoot 'rustup'
    $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
}
$taskVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$taskVsPath = if (Test-Path -LiteralPath $taskVsWhere) { & $taskVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath }
if (-not $taskVsPath -and (Test-Path -LiteralPath (Join-Path $taskToolchainRoot 'msvc\Common7\Tools\VsDevCmd.bat'))) { $taskVsPath = Join-Path $taskToolchainRoot 'msvc' }
if ($taskVsPath) {
    $taskDevCmd = Join-Path $taskVsPath 'Common7\Tools\VsDevCmd.bat'
    $taskEnvOutput = & $env:ComSpec /d /s /c "`"$taskDevCmd`" -no_logo -arch=x64 -host_arch=x64 >nul && set"
    foreach ($taskLine in $taskEnvOutput) {
        if ($taskLine -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process') }
    }
}
