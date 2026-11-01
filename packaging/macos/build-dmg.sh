#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) "Space Detective.app" and wraps it in a disk image:
#   dist/SpaceDetective-<version>.dmg
# Run on a Mac with Rust and the Xcode command line tools installed.
#
# Without an Apple Developer ID the app is only ad-hoc signed, and people have to allow it in
# System Settings › Privacy & Security the first time. To sign and notarize instead, set:
#   MACOS_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
#   MACOS_NOTARY_PROFILE=<name saved with `xcrun notarytool store-credentials`>
set -euo pipefail

cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
APP_NAME="Space Detective"
BIN=space-detective
APP="dist/$APP_NAME.app"
DMG="dist/SpaceDetective-$VERSION.dmg"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=11.0

# Swap this Mac's folder paths (which include the user name) for neutral ones in the app.
FLAGS=("--remap-path-prefix=$HOME=~")
if [[ -n "${CARGO_HOME:-}" ]]; then
    FLAGS+=("--remap-path-prefix=$CARGO_HOME=cargo")
fi
FLAGS+=("--remap-path-prefix=$PWD=.")
CARGO_ENCODED_RUSTFLAGS=$(IFS=$'\x1f'; echo "${FLAGS[*]}")
export CARGO_ENCODED_RUSTFLAGS
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/$BIN" \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN"
cp assets/icon.icns "$APP/Contents/Resources/AppIcon.icns"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY" "$APP"
else
    codesign --force --sign - "$APP"
fi

# The disk image holds the app and a shortcut to Applications to drag it onto.
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$DMG"
    if [[ -n "${MACOS_NOTARY_PROFILE:-}" ]]; then
        xcrun notarytool submit "$DMG" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
        xcrun stapler staple "$DMG"
    fi
fi

echo "Built $DMG"
