# Make the phone app's release key (once). Keep the file and its password safe and backed up:
# phones only accept an update signed with the same key as the app they have.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release\make-keys.ps1
#
# The desktop's update key is made separately (also once):
#   cd apps\desktop; npx tauri signer generate -w $HOME\.outpost\updater.key

$ErrorActionPreference = "Stop"
$dir = Join-Path $HOME ".outpost"
New-Item -ItemType Directory -Force $dir | Out-Null
$keystore = Join-Path $dir "outpost-release.keystore"
if (Test-Path $keystore) {
    Write-Host "There's already a phone release key at $keystore. Not making another."
    exit 0
}

# keytool comes with Java (Android Studio's, or the JDK the phone build uses).
$keytool = $null
foreach ($jdk in @($env:JAVA_HOME, "C:\Program Files\Android\Android Studio\jbr") + (Get-ChildItem "C:\Program Files\Microsoft", "C:\Program Files\Eclipse Adoptium", "C:\Program Files\Java" -Directory -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })) {
    if ($jdk -and (Test-Path (Join-Path $jdk "bin\keytool.exe"))) { $keytool = Join-Path $jdk "bin\keytool.exe"; break }
}
if (-not $keytool) { throw "keytool wasn't found. Install a JDK (winget install Microsoft.OpenJDK.17) and try again." }

Write-Host "Making the phone app's release key. Choose a password (at least 6 characters) and keep it safe."
& $keytool -genkeypair -v -keystore $keystore -alias outpost -keyalg RSA -keysize 4096 -validity 10000 `
    -dname "CN=Outpost, O=OutpostAI"
if ($LASTEXITCODE -ne 0) { throw "keytool failed." }
Write-Host ""
Write-Host "Saved: $keystore"
Write-Host "Back it up (with its password) somewhere safe, e.g. a password manager or a USB stick."
