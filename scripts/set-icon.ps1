# Replace Siksik's app icon everywhere (installer, shortcuts, taskbar, in-app logo).
# Usage: .\scripts\set-icon.ps1 C:\path\to\icon.png   (square PNG, ideally 1024x1024)
param([Parameter(Mandatory = $true)][string]$Icon)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Icon)) { Write-Error "File not found: $Icon" }
$Icon = (Resolve-Path $Icon).Path
Set-Location (Join-Path $PSScriptRoot "..")
npx tauri icon "$Icon" -o src-tauri/icons
Remove-Item -Recurse -Force src-tauri/icons/android, src-tauri/icons/ios -ErrorAction SilentlyContinue
Copy-Item src-tauri/icons/128x128@2x.png src/logo.png -Force
Write-Host "Icon updated. Rebuild with: npm run build"
