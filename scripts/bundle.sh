#!/bin/zsh
# Builds "BPM Detector.app" (release binary, icon, ad-hoc signature) and installs it in /Applications.
#   --universal   one binary for both Apple Silicon and Intel Macs
#   --no-install  only build the app in target/bundle
set -euo pipefail

cd "${0:A:h}/.."
export PATH="$HOME/.cargo/bin:$PATH"

universal=false
install=true
for option in "$@"; do
  case "$option" in
    --universal) universal=true ;;
    --no-install) install=false ;;
    *) echo "Unknown option: $option" >&2; exit 1 ;;
  esac
done

app_name="BPM Detector"
build_dir="target/bundle"
app="$build_dir/$app_name.app"
iconset="$build_dir/AppIcon.iconset"
binary="$app/Contents/MacOS/bpm-detector"

rm -rf "$app" "$iconset"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$iconset"

if $universal; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  lipo -create -output "$binary" \
    target/aarch64-apple-darwin/release/bpm-detector \
    target/x86_64-apple-darwin/release/bpm-detector
else
  cargo build --release
  cp target/release/bpm-detector "$binary"
fi
cp packaging/Info.plist "$app/Contents/Info.plist"

swiftc -O -o "$build_dir/make_icon" packaging/make_icon.swift
"$build_dir/make_icon" "$build_dir/icon_1024.png"
for size in 16 32 128 256 512; do
  sips -z $size $size "$build_dir/icon_1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$build_dir/icon_1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

codesign --force --sign - "$app"

if $install; then
  rm -rf "/Applications/$app_name.app"
  cp -R "$app" /Applications/
  echo "Installed /Applications/$app_name.app"
else
  echo "Built $app"
fi
