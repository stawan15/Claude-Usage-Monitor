#!/usr/bin/env bash
# Builds a release ClaudeMonitor.app (menu bar only, no Dock icon) into ./build.
set -euo pipefail
cd "$(dirname "$0")/.."

swift build -c release
BIN="$(swift build -c release --show-bin-path)/ClaudeMonitor"
APP="build/ClaudeMonitor.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/ClaudeMonitor"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Claude Monitor</string>
    <key>CFBundleDisplayName</key><string>Claude Monitor</string>
    <key>CFBundleIdentifier</key><string>local.claude-monitor</string>
    <key>CFBundleExecutable</key><string>ClaudeMonitor</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so macOS will run it and SMAppService (launch at login) accepts it.
codesign --force --sign - "$APP"
echo "Built $APP"
echo "Install: cp -R $APP /Applications/ && open /Applications/ClaudeMonitor.app"
