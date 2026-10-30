# One command for a whole Outpost release: build, commit the version change, push, publish.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release\ship.ps1 -Notes "What's new..."
#
# -Version 0.2.0      the version (default: the next patch, e.g. 0.1.11 -> 0.1.12)
# -Commit "message"   commit (and push) uncommitted changes first, with this message
# -SkipPhone / -SkipLinux   passed on to release.ps1
#
# It stops at the first problem; running it again with the same version carries on
# (release.ps1 reuses what it already built).

param(
    [string]$Version = "",
    [string]$Notes = "",
    [string]$Commit = "",
    [switch]$SkipPhone,
    [switch]$SkipLinux
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$attribution = @(
    "-m", "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>",
    "-m", "Claude-Session: https://claude.ai/code/session_01FHR5gRyQFyLM1MATPFBpEv"
)
function Invoke-Git([string[]]$GitArgs) {
    & git @GitArgs
    if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed." }
}

# 1. Changes not committed yet.
$pending = git status --porcelain
if ($pending) {
    if (-not $Commit) {
        Write-Host "There are uncommitted changes:" -ForegroundColor Yellow
        $pending | ForEach-Object { Write-Host "  $_" }
        throw "Commit them first, or run this with -Commit `"what changed`"."
    }
    Write-Host "== Committing: $Commit" -ForegroundColor Cyan
    Invoke-Git @("add", "-A")
    Invoke-Git (@("commit", "-m", $Commit) + $attribution)
    Invoke-Git @("push")
}

# 2. The version: given, or the next patch after the current one.
$current = (Get-Content (Join-Path $root "apps\desktop\src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json).version
if (-not $Version) {
    # Is the current version on GitHub already? (gh's "not found" isn't an error here.)
    $published = $false
    if (Get-Command gh -ErrorAction SilentlyContinue) {
        $ErrorActionPreference = "Continue"
        gh release view "v$current" --repo danite1252-ctrl/OutpostAI *> $null
        $published = ($LASTEXITCODE -eq 0)
        $ErrorActionPreference = "Stop"
    } else {
        throw "The gh tool isn't installed (winget install GitHub.cli); give the version with -Version."
    }
    if ($published) {
        $parts = $current.Split("-")[0].Split(".")
        $Version = "$($parts[0]).$($parts[1]).$([int]$parts[2] + 1)"
    } else {
        # The current version was never published (an earlier try stopped): finish it.
        $Version = $current
    }
}
if (-not $Notes) { $Notes = "Outpost $Version" }
Write-Host "== Releasing Outpost $Version" -ForegroundColor Cyan

$extra = @()
if ($SkipPhone) { $extra += "-SkipPhone" }
if ($SkipLinux) { $extra += "-SkipLinux" }
$release = Join-Path $PSScriptRoot "release.ps1"

# 3. Build (asks for the key passwords).
& powershell -ExecutionPolicy Bypass -File $release -Version $Version -Notes $Notes @extra
if ($LASTEXITCODE -ne 0) { throw "The build stopped. Fix the problem above and run this again with -Version $Version." }

# 4. The version change it made.
if (git status --porcelain) {
    Write-Host "== Committing the version change" -ForegroundColor Cyan
    Invoke-Git @("add", "-A")
    Invoke-Git (@("commit", "-m", "Outpost $Version") + $attribution)
}
Invoke-Git @("push")

# 5. Publish.
& powershell -ExecutionPolicy Bypass -File $release -Version $Version -Notes $Notes -PublishOnly @extra
if ($LASTEXITCODE -ne 0) { throw "Publishing stopped. Run this again with -Version $Version to retry." }

Write-Host ""
Write-Host "Outpost $Version is out." -ForegroundColor Green
Write-Host "Servers and desktop apps see it within a day (or press Check now); phones follow."
