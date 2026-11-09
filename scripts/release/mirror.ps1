# Put a built release's files on Outpost's own download server too, in place of the last
# release's: the phone app, the Windows installer, the Linux server, install.sh and a
# latest.json pointing at them. Apps still learn about new releases from GitHub, then
# download from here first (checking every file's signature or checksum) and from GitHub
# when this copy is missing or out of date.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release\mirror.ps1 -Version 1.5.6
#
# (release.ps1 runs it before publishing on GitHub.) Where to put them comes from
# $HOME\.outpost\mirror.json, which never goes in the repo:
#
#   { "url": "https://outpost.hartspace.cc/downloads",
#     "host": "192.168.1.32", "user": "you",
#     "folder": "/var/www/hartspace.cc/public_html/outpost/downloads",
#     "key": "C:\\Users\\you\\.ssh\\server.ppk" }
#
# "key" is optional: without it, PuTTY's Pageant must have the key loaded. With it, psftp
# asks for the key's passphrase once (or uses Pageant if it already has it). Needs PuTTY's
# psftp (it comes with PuTTY).

param(
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$config = Join-Path $HOME ".outpost\mirror.json"
$out = Join-Path $root "target\release-out\v$Version"
$stage = Join-Path $root "target\release-out\v$Version-mirror"

function Mirror-Config {
    if (-not (Test-Path $config)) { return $null }
    $c = Get-Content $config -Raw | ConvertFrom-Json
    foreach ($k in @("url", "host", "user", "folder")) {
        if (-not $c.$k) { throw "$config has no `"$k`"." }
    }
    $c.url = $c.url.TrimEnd("/")
    $c.folder = $c.folder.TrimEnd("/")
    return $c
}

$c = Mirror-Config
if (-not $c) {
    Write-Host "No $config, so the files only go on GitHub." -ForegroundColor Yellow
    exit 0
}
if (-not (Test-Path (Join-Path $out "latest.json"))) { throw "Nothing built for v$Version yet." }

$psftp = (Get-Command psftp -ErrorAction SilentlyContinue).Source
if (-not $psftp) {
    foreach ($p in @("$env:ProgramFiles\PuTTY\psftp.exe", "${env:ProgramFiles(x86)}\PuTTY\psftp.exe")) {
        if (Test-Path $p) { $psftp = $p; break }
    }
}
if (-not $psftp) { throw "PuTTY's psftp wasn't found (it comes with PuTTY: winget install PuTTY.PuTTY)." }

# The files, under names that stay the same from release to release.
$manifest = Get-Content (Join-Path $out "latest.json") -Raw | ConvertFrom-Json
$files = [ordered]@{}
$win = Get-ChildItem $out -Filter "Outpost_$($Version)_x64-setup.exe" | Select-Object -First 1
if ($win) {
    $files["Outpost-setup.exe"] = $win.FullName
    $manifest.platforms."windows-x86_64".url = "$($c.url)/Outpost-setup.exe"
}
$linux = Join-Path $out "outpost-gateway-linux-x86_64"
if (Test-Path $linux) {
    $files["outpost-gateway-linux-x86_64"] = $linux
    $manifest.platforms."linux-x86_64".url = "$($c.url)/outpost-gateway-linux-x86_64"
}
$apk = Join-Path $out "Outpost-$Version.apk"
if (Test-Path $apk) {
    $files["Outpost.apk"] = $apk
    $manifest.mobile.url = "$($c.url)/Outpost.apk"
}
$install = Join-Path $out "install.sh"
if (Test-Path $install) { $files["install.sh"] = $install }

Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
[IO.File]::WriteAllText((Join-Path $stage "latest.json"), ($manifest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))

# One session (one passphrase): the folder (made the first time; "already there" is fine
# after that), then each file over the last release's, latest.json last so it never points at
# files from before. Each file is checked by the apps before use, so one caught halfway is
# just skipped for GitHub's copy.
$lines = @("mkdir `"$($c.folder)`"", "cd `"$($c.folder)`"")
foreach ($name in $files.Keys) { $lines += "put `"$($files[$name])`" `"$name`"" }
$lines += "put `"$(Join-Path $stage "latest.json")`" `"latest.json`""
$lines += "quit"
$script = Join-Path $stage "psftp.txt"
[IO.File]::WriteAllLines($script, $lines, (New-Object Text.UTF8Encoding $false))

Write-Host ""
Write-Host "== Copying v$Version to $($c.host) ($($c.url))" -ForegroundColor Cyan
$psftpArgs = @("-b", $script, "-be")
if ($c.key) { $psftpArgs += @("-i", $c.key) }
$psftpArgs += "$($c.user)@$($c.host)"
& $psftp @psftpArgs
if ($LASTEXITCODE -ne 0) { throw "Copying to $($c.host) didn't finish (psftp stopped with $LASTEXITCODE)." }
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "On the download server: $($files.Keys -join ', '), latest.json"
