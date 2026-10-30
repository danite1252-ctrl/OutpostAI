# Build an Outpost release: the Windows installer (signed for the updater), the Linux server
# (built on the server by its runner, signed with the same key), the phone app (signed with
# the release key) and latest.json, ready to put on GitHub Releases.
#
#   1. Build:    powershell -ExecutionPolicy Bypass -File scripts\release\release.ps1 -Version 0.2.0 -Notes "What's new..."
#   2. Commit and push the version change it made.
#   3. Publish:  powershell -ExecutionPolicy Bypass -File scripts\release\release.ps1 -Version 0.2.0 -Notes "What's new..." -PublishOnly
#
# Needs, once: the update key ($HOME\.outpost\updater.key, from `npx tauri signer generate`)
# and the phone key ($HOME\.outpost\outpost-release.keystore, from make-keys.ps1).
# See docs\releasing.md.

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Notes = "",
    [switch]$Publish,
    [switch]$PublishOnly,
    [switch]$SkipPhone,
    [switch]$SkipLinux
)

$ErrorActionPreference = "Stop"
$repo = "danite1252-ctrl/OutpostAI"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$keys = Join-Path $HOME ".outpost"
$updaterKey = Join-Path $keys "updater.key"
$keystore = Join-Path $keys "outpost-release.keystore"
$tag = "v$Version"
$out = Join-Path $root "target\release-out\$tag"

function Step($text) { Write-Host ""; Write-Host "== $text" -ForegroundColor Cyan }
function Run($what) {
    $exe = $what[0]
    # npm.ps1 / npx.ps1 misread their arguments when called this way; use the .cmd versions.
    if ($exe -eq "npm" -or $exe -eq "npx") { $exe = "$exe.cmd" }
    $rest = @($what | Select-Object -Skip 1)
    & $exe @rest
    if ($LASTEXITCODE -ne 0) { throw "Failed: $($what -join ' ')" }
}
function Plain($secure) {
    [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
}

if ($Publish -and -not $PublishOnly) { throw "Build first (without -Publish), commit and push the version change, then run again with -PublishOnly." }
if ($PublishOnly) {
    $Publish = $true
    if (-not (Test-Path (Join-Path $out "latest.json"))) { throw "Nothing built for $tag yet: run this without -PublishOnly first." }
    Set-Location $root
    if (git status --porcelain) { throw "Commit (and push) the version change first: git add -A; git commit -m `"Outpost $Version`"; git push" }
}
if (-not $PublishOnly) {

if (-not (Test-Path $updaterKey)) { throw "No update key at $updaterKey. Make it first (docs\releasing.md)." }
if (-not $SkipPhone -and -not (Test-Path $keystore)) { throw "No phone release key at $keystore. Run scripts\release\make-keys.ps1 first, or add -SkipPhone." }

Set-Location $root
# Version files may already be changed by an earlier try of this release.
$dirty = git status --porcelain | Where-Object { $_ -notmatch "Cargo\.(toml|lock)|package(-lock)?\.json|tauri\.conf\.json|app\.json|version\.ts" }
if ($dirty) { throw "There are uncommitted changes. Commit them first, so the release matches the code." }

$conf = Get-Content (Join-Path $root "apps\desktop\src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
if ($conf.plugins.updater.pubkey -like "PENDING*") { throw "The update key's public half isn't in tauri.conf.json yet (plugins.updater.pubkey). See docs\releasing.md." }

Step "Version $Version"
$current = (Get-Content (Join-Path $root "apps\desktop\src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json).version
if ($current -ne $Version) { Run @("node", "scripts\release\bump.mjs", $Version) }

$updaterPassword = Plain (Read-Host "Update key password" -AsSecureString)
if (-not $SkipPhone) { $phonePassword = Plain (Read-Host "Phone release key password" -AsSecureString) }

Step "Gateway (for apps that start Outpost's MCP bridge)"
Run @("cargo", "build", "--release", "-p", "outpost-gateway")
$bin = Join-Path $root "apps\desktop\src-tauri\binaries"
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item (Join-Path $root "target\release\outpost-gateway.exe") (Join-Path $bin "outpost-gateway-x86_64-pc-windows-msvc.exe") -Force

Step "Desktop installer"
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $updaterKey -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $updaterPassword
Run @("npm", "install", "--no-audit", "--no-fund")
Run @("npm", "run", "tauri", "-w", "@outpost/desktop", "--", "build", "--config", "src-tauri/tauri.release.conf.json")
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
$nsis = Join-Path $root "target\release\bundle\nsis"
$setup = Get-ChildItem $nsis -Filter "Outpost_$($Version)_x64-setup.exe" | Select-Object -First 1
if (-not $setup) { throw "The installer wasn't made (looked in $nsis)." }
$sig = Get-Content "$($setup.FullName).sig" -Raw

New-Item -ItemType Directory -Force $out | Out-Null
$setupName = "Outpost_$($Version)_x64-setup.exe"
Copy-Item $setup.FullName (Join-Path $out $setupName) -Force
Copy-Item "$($setup.FullName).sig" (Join-Path $out "$setupName.sig") -Force

$linuxName = "outpost-gateway-linux-x86_64"
if (-not $SkipLinux) {
    Step "Linux server (built on the server by its runner)"
    # The runner (scripts/linux/runner.sh) watches target\overnight-linux in the shared folder.
    $flags = Join-Path $root "target\overnight-linux"
    $linuxOut = Join-Path $flags "release-out"
    $built = Join-Path $linuxOut $linuxName
    $versionFile = Join-Path $linuxOut "version.txt"
    $result = Join-Path $flags "linux-result.txt"
    Remove-Item -Recurse -Force $linuxOut -ErrorAction SilentlyContinue
    New-Item -ItemType File -Force (Join-Path $flags "release") | Out-Null
    Write-Host "Waiting for the server's runner to build it (a few minutes; up to 30)..."
    $deadline = (Get-Date).AddMinutes(30)
    while (-not ((Test-Path $built) -and (Test-Path $versionFile))) {
        if ((Test-Path $result) -and (Select-String -Path $result -Pattern "=== release finished .*ok: false" -Quiet)) {
            throw "The server couldn't build it: see $result. Fix it, or add -SkipLinux."
        }
        if ((Get-Date) -gt $deadline) { throw "The server's runner didn't answer. Is it running? Or add -SkipLinux." }
        Start-Sleep -Seconds 10
    }
    $linuxVersion = (Get-Content $versionFile -Raw).Trim()
    if ($linuxVersion -ne $Version) { throw "The server built version $linuxVersion, not $Version. Is the shared folder up to date?" }
    New-Item -ItemType Directory -Force $out | Out-Null
    Copy-Item $built (Join-Path $out $linuxName) -Force
    # Signed with the update key, like the installer: servers check it before installing.
    Run @("npm", "run", "tauri", "-w", "@outpost/desktop", "--", "signer", "sign", "-f", $updaterKey, "-p", $updaterPassword, (Join-Path $out $linuxName))
    $linuxSig = (Get-Content (Join-Path $out "$linuxName.sig") -Raw).Trim()
    $linuxHash = (Get-FileHash (Join-Path $out $linuxName) -Algorithm SHA256).Hash.ToLower()
    Copy-Item (Join-Path $root "scripts\linux\install.sh") (Join-Path $out "install.sh") -Force
}

$apkName = "Outpost-$Version.apk"
if (-not $SkipPhone) {
    Step "Phone app"
    # Android Studio's SDK, and a Java 17 or 21 for Gradle (newer ones fail), like the test runner.
    if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA "Android\Sdk" }
    if (-not (Test-Path $env:ANDROID_HOME)) { throw "The Android SDK wasn't found at $env:ANDROID_HOME (install Android Studio), or add -SkipPhone." }
    $jdks = @()
    foreach ($base in @("C:\Program Files\Microsoft", "C:\Program Files\Eclipse Adoptium", "C:\Program Files\Java", "C:\Program Files\Zulu")) {
        if (Test-Path $base) { $jdks += Get-ChildItem $base -Directory | ForEach-Object { $_.FullName } }
    }
    $jdks += "C:\Program Files\Android\Android Studio\jbr"
    if ($env:JAVA_HOME) { $jdks = @($env:JAVA_HOME) + $jdks }
    $jdk = $jdks | Where-Object {
        $rel = Join-Path $_ "release"
        (Test-Path $rel) -and (Select-String -Path $rel -Pattern '^JAVA_VERSION="(17|21)' -Quiet)
    } | Select-Object -First 1
    if (-not $jdk) { throw "No Java 17 or 21 found for the phone build (winget install Microsoft.OpenJDK.17), or add -SkipPhone." }
    $env:JAVA_HOME = $jdk
    $mobile = Join-Path $root "apps\mobile"
    Set-Location $mobile
    $env:CI = "true"
    Run @("npm", "install", "--no-audit", "--no-fund")
    Run @("npx", "expo", "prebuild", "--platform", "android", "--no-install")
    # Tell Gradle where the SDK is, whatever the environment says.
    $sdkDir = ($env:ANDROID_HOME -replace "\\", "/")
    [IO.File]::WriteAllText((Join-Path $mobile "android\local.properties"), "sdk.dir=$sdkDir`n")
    $env:OUTPOST_KEYSTORE = $keystore
    $env:OUTPOST_KEYSTORE_PASSWORD = $phonePassword
    Set-Location (Join-Path $mobile "android")
    Run @(".\gradlew.bat", "assembleRelease", "--console=plain")
    Remove-Item Env:OUTPOST_KEYSTORE, Env:OUTPOST_KEYSTORE_PASSWORD
    Set-Location $root
    Copy-Item (Join-Path $mobile "android\app\build\outputs\apk\release\app-release.apk") (Join-Path $out $apkName) -Force
}

Step "latest.json"
$download = "https://github.com/$repo/releases/download/$tag"
$manifest = [ordered]@{
    version   = $Version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{ signature = $sig.Trim(); url = "$download/$setupName" }
    }
}
if (-not $SkipLinux) {
    $manifest.platforms."linux-x86_64" = [ordered]@{ signature = $linuxSig; url = "$download/$linuxName"; sha256 = $linuxHash }
}
if (-not $SkipPhone) {
    # The phone checks the download against this before installing it.
    $apkHash = (Get-FileHash (Join-Path $out $apkName) -Algorithm SHA256).Hash.ToLower()
    $manifest.mobile = [ordered]@{ version = $Version; url = "$download/$apkName"; sha256 = $apkHash }
}
# UTF-8 without a byte-order mark (GitHub serves it as-is).
[IO.File]::WriteAllText((Join-Path $out "latest.json"), ($manifest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))

Step "Built"
Get-ChildItem $out | ForEach-Object { Write-Host "  $($_.Name)  ($([math]::Round($_.Length / 1MB, 1)) MB)" }

$changed = git status --porcelain
if ($changed -and -not $Publish) {
    Write-Host ""
    Write-Host "Next: commit and push the version change, then publish:" -ForegroundColor Yellow
    Write-Host "  git add -A; git commit -m `"Outpost $Version`"; git push"
    Write-Host "  powershell -ExecutionPolicy Bypass -File scripts\release\release.ps1 -Version $Version -Notes `"...`" -PublishOnly"
}

} # end of the build

if ($Publish) {
    Step "Publishing $tag on github.com/$repo"
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "The gh tool isn't installed (winget install GitHub.cli), so publish by hand: see docs\releasing.md." }
    $files = Get-ChildItem $out | ForEach-Object { $_.FullName }
    $notesText = if ($Notes) { $Notes } else { "Outpost $Version" }
    # The tag goes on the commit you're on (pushed to GitHub).
    $commit = (git rev-parse HEAD).Trim()
    Run (@("gh", "release", "create", $tag, "--repo", $repo, "--target", $commit, "--title", "Outpost $Version", "--notes", $notesText) + $files)
    Write-Host "Published. Installed copies of Outpost will see it within a day (or press Check now)."
} else {
    Write-Host ""
    Write-Host "To publish: make a release called $tag on https://github.com/$repo/releases/new and attach every file in"
    Write-Host "  $out"
    Write-Host "(or run this again with -Publish once the gh tool is set up)."
}
