#!/bin/zsh
# Builds "BPM Detector.app" (release binary, icon, ad-hoc signature) and installs it in /Applications.
set -euo pipefail

cd "${0:A:h}/.."
export PATH="$HOME/.cargo/bin:$PATH"

app_name="BPM Detector"
build_dir="target/bundle"
app="$build_dir/$app_name.app"
iconset="$build_dir/AppIcon.iconset"

cargo build --release

rm -rf "$app" "$iconset"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$iconset"
cp target/release/bpm-detector "$app/Contents/MacOS/bpm-detector"
cp packaging/Info.plist "$app/Contents/Info.plist"

swiftc -O -o "$build_dir/make_icon" packaging/make_icon.swift
"$build_dir/make_icon" "$build_dir/icon_1024.png"
for size in 16 32 128 256 512; do
  sips -z $size $size "$build_dir/icon_1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$build_dir/icon_1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

codesign --force --sign - "$app"

rm -rf "/Applications/$app_name.app"
cp -R "$app" /Applications/
echo "Installed /Applications/$app_name.app"
