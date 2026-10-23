# Phase 1 spike: install ostris/ai-toolkit beside Outpost (not inside it) to test
# LoRA training on this PC's GPU. Uses the uv that Outpost already downloaded, so no
# system Python is touched. Safe to run again: it updates and re-installs.
#
#   powershell -ExecutionPolicy Bypass -File .\scripts\spike\install-ai-toolkit.ps1
#
# Then start the UI with: .\scripts\spike\start-ai-toolkit.ps1

param(
    [string]$Target = "C:\dev\ai-toolkit"
)
$ErrorActionPreference = "Stop"

function Find-Uv {
    $root = Join-Path $env:LOCALAPPDATA "Outpost\engines\uv"
    if (Test-Path $root) {
        $uv = Get-ChildItem -Path $root -Recurse -Filter uv.exe -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($uv) { return $uv.FullName }
    }
    $onPath = Get-Command uv -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    throw "uv not found. Install ComfyUI from Outpost (Settings > Engines) first; it downloads uv."
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { throw "git is needed: https://git-scm.com/download/win" }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { throw "Node.js 20+ is needed for the ai-toolkit UI." }

# Run a program; stop with a clear message if it fails (PowerShell doesn't for programs).
function Invoke-Step([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE). See the output above." }
}

$uv = Find-Uv
Write-Host "Using uv at $uv" -ForegroundColor Yellow
# Share Outpost's uv caches so Python and wheels download once.
$uvRoot = Join-Path $env:LOCALAPPDATA "Outpost\engines\uv"
$env:UV_PYTHON_INSTALL_DIR = Join-Path $uvRoot "python"
$env:UV_CACHE_DIR = Join-Path $uvRoot "cache"

if (Test-Path (Join-Path $Target ".git")) {
    Write-Host "Updating $Target"
    Invoke-Step "git pull" { git -C $Target pull --ff-only }
} else {
    Write-Host "Cloning ai-toolkit into $Target"
    Invoke-Step "git clone" { git clone https://github.com/ostris/ai-toolkit.git $Target }
}
Invoke-Step "git submodule update" { git -C $Target submodule update --init --recursive }

Set-Location $Target
$py = Join-Path $Target "venv\Scripts\python.exe"
if (-not (Test-Path $py)) {
    Invoke-Step "Creating the Python 3.12 environment" { & $uv venv --python 3.12 venv }
}

Write-Host "Installing PyTorch (CUDA 13.0) - this is a few GB" -ForegroundColor Yellow
# The versions ai-toolkit's README pins. If uv reports a version conflict, drop
# torchaudio from this line and run the script again.
Invoke-Step "Installing PyTorch" { & $uv pip install --python $py torch==2.13.0 torchvision==0.28.0 torchaudio==2.11.0 --index-url https://download.pytorch.org/whl/cu130 }
Write-Host "Installing ai-toolkit requirements" -ForegroundColor Yellow
Invoke-Step "Installing requirements" { & $uv pip install --python $py -r requirements.txt }

Write-Host "Installing the UI's packages" -ForegroundColor Yellow
Set-Location (Join-Path $Target "ui")
Invoke-Step "npm install" { npm install }

& $py -c "import torch; print('torch', torch.__version__, 'cuda', torch.cuda.is_available(), torch.cuda.get_device_name(0) if torch.cuda.is_available() else '')"
Write-Host "Done. Start it with .\scripts\spike\start-ai-toolkit.ps1 (from C:\dev\Outpost)" -ForegroundColor Green
