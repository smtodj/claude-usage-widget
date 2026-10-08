#!/bin/sh
# Builds the macOS desktop widget (UsageWidget.appex) and the helper that
# asks it to redraw (widget-reload) into macos/build/, where
# tauri.conf.json's bundle.macOS.files picks them up. Needs Xcode; installs
# XcodeGen with Homebrew if it is missing.
set -eu

cd "$(dirname "$0")/widget"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' ../../Cargo.toml | head -n 1)

command -v xcodegen >/dev/null 2>&1 || brew install xcodegen
xcodegen generate --quiet

xcodebuild \
  -project ClaudeUsageWidget.xcodeproj \
  -configuration Release \
  -target UsageWidget -target widget-reload \
  ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO \
  MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$version" \
  SYMROOT="$PWD/build/xcode" \
  -quiet build

out=../build
rm -rf "$out"
mkdir -p "$out"
cp -R build/xcode/Release/UsageWidget.appex "$out/"
cp build/xcode/Release/widget-reload "$out/"

# The widget only loads when sandboxed and signed.
codesign --verify --strict --verbose "$out/UsageWidget.appex"
codesign -d --entitlements - "$out/UsageWidget.appex" 2>/dev/null | grep -q app-sandbox
codesign --verify --strict "$out/widget-reload"
test "$(plutil -extract CFBundleVersion raw "$out/UsageWidget.appex/Contents/Info.plist")" = "$version"
lipo -archs "$out/UsageWidget.appex/Contents/MacOS/UsageWidget"
echo "widget built: $out/UsageWidget.appex (v$version)"
