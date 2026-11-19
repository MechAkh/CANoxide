<#
.SYNOPSIS
    Silent per-user install / reinstall / uninstall test of the CANoxide setup.

.DESCRIPTION
    Needs target\installer\CANoxide-<version>-setup.exe (run build.ps1 first).
    Installs for the current user only (no elevation) into a new folder under
    target\installtest, with the "addtopath" task, and checks the files, the
    uninstall registry entry, the user PATH and "canoxide-cli version".
    Installs a second time, then uninstalls silently and checks that the
    files, the registry entry and the PATH entry are gone and that the user
    PATH is byte-identical to its value before the test (restored if not).
    Never starts the GUI. No window opens. Exit code 0 = all checks passed.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$AppId = '{089894BC-3DF1-4DCF-BB97-241F806CA212}'
$UninstKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${AppId}_is1"

$script:failures = 0
function Check([bool]$Ok, [string]$What) {
    if ($Ok) { Write-Host "PASS  $What" }
    else { Write-Host "FAIL  $What"; $script:failures++ }
}

function Get-UserPath {
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
    try {
        if ($null -eq $k) { return @{ Exists = $false; Value = $null; Kind = $null } }
        $names = $k.GetValueNames()
        if ($names -notcontains 'Path') { return @{ Exists = $false; Value = $null; Kind = $null } }
        return @{
            Exists = $true
            Value  = [string]$k.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            Kind   = $k.GetValueKind('Path')
        }
    } finally { if ($k) { $k.Close() } }
}

function Set-UserPath($Saved) {
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try {
        if ($Saved.Exists) { $k.SetValue('Path', $Saved.Value, $Saved.Kind) }
        elseif ($k.GetValueNames() -contains 'Path') { $k.DeleteValue('Path') }
    } finally { $k.Close() }
}

function Count-InPath([string]$PathValue, [string]$Dir) {
    if (-not $PathValue) { return 0 }
    $want = $Dir.TrimEnd('\')
    @($PathValue.Split(';') | Where-Object { $_.Trim().TrimEnd('\') -ieq $want }).Count
}

function Same-Path($A, $B) {
    ($A.Exists -eq $B.Exists) -and ($A.Kind -eq $B.Kind) -and
        ([string]::CompareOrdinal([string]$A.Value, [string]$B.Value) -eq 0)
}

# Runs a program hidden and waits for it and its child processes.
function Run-Hidden([string]$Exe, [string[]]$ArgList) {
    $p = Start-Process -FilePath $Exe -ArgumentList $ArgList -WindowStyle Hidden -Wait -PassThru
    return $p.ExitCode
}

function Wait-Until([scriptblock]$Cond, [int]$Seconds) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $Cond) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return (& $Cond)
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$version = $null; $section = ''
foreach ($line in Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml')) {
    $t = $line.Trim()
    if ($t -match '^\[(.+)\]$') { $section = $Matches[1].Trim(); continue }
    if ($section -eq 'package' -and $t -match '^version\s*=\s*"([^"]+)"') { $version = $Matches[1]; break }
}
$setup = Join-Path $repoRoot "target\installer\CANoxide-$version-setup.exe"
if (-not (Test-Path -LiteralPath $setup)) {
    [Console]::Error.WriteLine("setup exe not found: $setup (run build.ps1 first)")
    exit 2
}
if (Test-Path -LiteralPath $UninstKey) {
    [Console]::Error.WriteLine("CANoxide is already installed for this user ($UninstKey); uninstall it first")
    exit 2
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$testRoot = Join-Path $repoRoot "target\installtest\run-$stamp"
$appDir = Join-Path $testRoot 'CANoxide'
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
$dataDir = Join-Path $env:APPDATA 'CANoxide'
$dataExisted = Test-Path -LiteralPath $dataDir

$savedPath = Get-UserPath
Write-Host "Setup:    $setup"
Write-Host "Install:  $appDir"
Write-Host "User PATH before: exists=$($savedPath.Exists) kind=$($savedPath.Kind) length=$(([string]$savedPath.Value).Length)"

$installed = $false
try {
    foreach ($round in 1, 2) {
        $log = Join-Path $testRoot "install$round.log"
        $code = Run-Hidden $setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/CURRENTUSER', '/NORESTART',
            "/DIR=`"$appDir`"", '/TASKS="addtopath"', "/LOG=`"$log`"")
        $installed = $true
        Check ($code -eq 0) "install $round exit code 0 (got $code)"
        foreach ($f in 'CANoxide.exe', 'canoxide-cli.exe', 'CANoxide.installed', 'LICENSE', 'README.md', 'docs\CLI.md') {
            Check (Test-Path -LiteralPath (Join-Path $appDir $f) -PathType Leaf) "install ${round}: $f exists"
        }
        $reg = Get-ItemProperty -LiteralPath $UninstKey -ErrorAction SilentlyContinue
        Check ($null -ne $reg) "install ${round}: HKCU uninstall entry exists"
        if ($reg) { Check ($reg.DisplayVersion -eq $version) "install ${round}: DisplayVersion = $version (got $($reg.DisplayVersion))" }
        $now = Get-UserPath
        $n = Count-InPath $now.Value $appDir
        Check ($n -eq 1) "install ${round}: user PATH holds the folder once (count $n)"
        Check ($now.Kind -eq $savedPath.Kind -or -not $savedPath.Exists) "install ${round}: user PATH kind kept ($($now.Kind))"

        if ($round -eq 1) {
            $cli = Join-Path $appDir 'canoxide-cli.exe'
            $out = Join-Path $testRoot 'cli_version.txt'
            $err = Join-Path $testRoot 'cli_version.err'
            # Console program: run it in this (hidden) console, output to files.
            $p = Start-Process -FilePath $cli -ArgumentList 'version' -NoNewWindow -Wait -PassThru `
                -RedirectStandardOutput $out -RedirectStandardError $err
            Check ($p.ExitCode -eq 0) "canoxide-cli version exit code 0 (got $($p.ExitCode))"
            Write-Host "      cli output: $((Get-Content -LiteralPath $out -Raw).Trim())"
        }
    }

    $uninst = Join-Path $appDir 'unins000.exe'
    Check (Test-Path -LiteralPath $uninst) 'uninstaller exists'
    $ulog = Join-Path $testRoot 'uninstall.log'
    $code = Run-Hidden $uninst @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$ulog`"")
    Check ($code -eq 0) "uninstall exit code 0 (got $code)"
    # The uninstaller runs from a temp copy: wait for it to finish.
    $gone = Wait-Until { -not (Test-Path -LiteralPath $UninstKey) -and -not (Test-Path -LiteralPath $uninst) } 60
    Check $gone 'uninstall finished within 60 s'
    if ($gone) { $installed = $false }
    foreach ($f in 'CANoxide.exe', 'canoxide-cli.exe', 'CANoxide.installed', 'LICENSE', 'README.md', 'docs\CLI.md', 'unins000.exe') {
        Check (-not (Test-Path -LiteralPath (Join-Path $appDir $f))) "uninstall: $f gone"
    }
    Check (-not (Test-Path -LiteralPath $UninstKey)) 'uninstall: HKCU uninstall entry gone'
    $after = Get-UserPath
    Check ((Count-InPath $after.Value $appDir) -eq 0) 'uninstall: PATH entry gone'
    Check (Same-Path $after $savedPath) 'uninstall: user PATH byte-identical to its value before the test'
    if ($dataExisted) { Check (Test-Path -LiteralPath $dataDir) 'silent uninstall kept %APPDATA%\CANoxide' }
} catch {
    Check $false "unexpected error: $($_.Exception.Message)"
} finally {
    if ($installed -and (Test-Path -LiteralPath (Join-Path $appDir 'unins000.exe'))) {
        Write-Host 'Cleanup: uninstalling the test copy'
        Run-Hidden (Join-Path $appDir 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') | Out-Null
        Wait-Until { -not (Test-Path -LiteralPath $UninstKey) } 60 | Out-Null
    }
    $final = Get-UserPath
    if (-not (Same-Path $final $savedPath)) {
        Write-Host 'Cleanup: user PATH differs from its saved value, restoring it'
        Set-UserPath $savedPath
        $script:failures++
    }
    # The CLI may create the data folder on its first run; remove it only when
    # this test created it and it holds nothing.
    if (-not $dataExisted -and (Test-Path -LiteralPath $dataDir)) {
        $items = @(Get-ChildItem -LiteralPath $dataDir -Force -Recurse -File)
        if ($items.Count -eq 0) {
            Remove-Item -LiteralPath $dataDir -Recurse -Force
            Write-Host 'Cleanup: removed the empty %APPDATA%\CANoxide that the test created'
        } else {
            Write-Host "Note: the test created $dataDir with $($items.Count) file(s); left in place"
        }
    }
}

if ($script:failures -eq 0) { Write-Host 'RESULT: all checks passed'; exit 0 }
Write-Host "RESULT: $($script:failures) check(s) failed"
exit 1
