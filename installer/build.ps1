<#
.SYNOPSIS
    Builds the CANoxide Windows setup program with Inno Setup 6.

.DESCRIPTION
    Reads the version from Cargo.toml, checks that the release exes exist,
    finds ISCC.exe and compiles installer\CANoxide.iss. Prints the path,
    size and SHA-256 of the setup exe. Exits non-zero on any failure.
    Windows PowerShell 5.1 compatible.

.PARAMETER Build
    Run "cargo build --release" first. Without it, the script only checks
    that target\release holds both exes.

.PARAMETER ExeDir
    Folder that holds CANoxide.exe and canoxide-cli.exe.
    Default: <repo>\target\release.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File installer\build.ps1 -Build
#>
[CmdletBinding()]
param(
    [switch]$Build,
    [string]$ExeDir
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    [Console]::Error.WriteLine("build.ps1: $Message")
    exit 1
}

try {
    $installerDir = $PSScriptRoot
    $repoRoot = Split-Path -Parent $installerDir
    $iss = Join-Path $installerDir 'CANoxide.iss'
    if (-not (Test-Path -LiteralPath $iss)) { Fail "script not found: $iss" }

    # Version from the [package] section of Cargo.toml.
    $cargoToml = Join-Path $repoRoot 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $cargoToml)) { Fail "Cargo.toml not found: $cargoToml" }
    $version = $null
    $section = ''
    foreach ($line in Get-Content -LiteralPath $cargoToml) {
        $t = $line.Trim()
        if ($t -match '^\[(.+)\]$') { $section = $Matches[1].Trim(); continue }
        if ($section -eq 'package' -and $t -match '^version\s*=\s*"([^"]+)"') {
            $version = $Matches[1]
            break
        }
    }
    if (-not $version) { Fail 'no version in the [package] section of Cargo.toml' }
    if ($version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$') { Fail "unexpected version: $version" }
    Write-Host "CANoxide version $version"

    if ($Build) {
        $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
        if (-not (Test-Path -LiteralPath $cargo)) {
            $cmd = Get-Command cargo.exe -ErrorAction SilentlyContinue
            if (-not $cmd) { Fail 'cargo.exe not found' }
            $cargo = $cmd.Source
        }
        Write-Host "Running cargo build --release"
        Push-Location $repoRoot
        try {
            & $cargo build --release
            $code = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        if ($code -ne 0) { Fail "cargo build failed (exit code $code)" }
    }

    if (-not $ExeDir) { $ExeDir = Join-Path $repoRoot 'target\release' }
    $ExeDir = [System.IO.Path]::GetFullPath($ExeDir)
    foreach ($exe in 'CANoxide.exe', 'canoxide-cli.exe') {
        $p = Join-Path $ExeDir $exe
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) {
            Fail "$p not found (run with -Build, or cargo build --release)"
        }
    }

    # ISCC: the per-user install, then Program Files (x86), then PATH.
    $iscc = $null
    $candidates = @((Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'))
    $pf86 = [Environment]::GetFolderPath('ProgramFilesX86')
    if ($pf86) { $candidates += (Join-Path $pf86 'Inno Setup 6\ISCC.exe') }
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c -PathType Leaf)) { $iscc = $c; break }
    }
    if (-not $iscc) {
        $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($cmd) { $iscc = $cmd.Source }
    }
    if (-not $iscc) { Fail 'ISCC.exe (Inno Setup 6) not found' }
    Write-Host "ISCC: $iscc"

    # ISCC is a console program: it runs in this console, no window opens.
    $outDir = Join-Path $repoRoot 'target\installer'
    $setup = Join-Path $outDir "CANoxide-$version-setup.exe"
    if (Test-Path -LiteralPath $setup) { Remove-Item -LiteralPath $setup -Force }
    $isccArgs = @("/DAppVersion=$version", "/DExeDir=$ExeDir", '/Q', $iss)
    $p = Start-Process -FilePath $iscc -ArgumentList ($isccArgs | ForEach-Object { '"' + $_ + '"' }) `
        -NoNewWindow -Wait -PassThru
    if ($p.ExitCode -ne 0) { Fail "ISCC failed (exit code $($p.ExitCode))" }
    if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { Fail "setup exe not found: $setup" }

    $item = Get-Item -LiteralPath $setup
    $hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash
    Write-Host "Setup:   $($item.FullName)"
    Write-Host "Size:    $($item.Length) bytes"
    Write-Host "SHA-256: $hash"
    exit 0
} catch {
    Fail $_.Exception.Message
}
