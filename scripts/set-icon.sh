#!/usr/bin/env sh
# Replace Siksik's app icon everywhere (installer, shortcuts, dock/taskbar, in-app logo).
# Usage: ./scripts/set-icon.sh path/to/icon.png      (square PNG, ideally 1024x1024)
set -e
if [ -z "$1" ] || [ ! -f "$1" ]; then
  echo "Usage: $0 path/to/icon.png  (square PNG or SVG, 1024x1024 recommended)"; exit 1
fi
cd "$(dirname "$0")/.."
npx tauri icon "$1" -o src-tauri/icons
rm -rf src-tauri/icons/android src-tauri/icons/ios
cp src-tauri/icons/128x128@2x.png src/logo.png
echo "Icon updated. Rebuild with: npm run build"
