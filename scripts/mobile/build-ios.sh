#!/usr/bin/env bash
# Builds yttt-mobile-core for iOS devices and the simulator, generates the
# Swift bindings, and packs both static libraries into an XCFramework.
#
# Output (under target/, not tracked):
#   target/mobile/ios/YtttMobileCore.xcframework
#   target/mobile/ios/swift/yttt_mobile_core.swift
#
# Usage: scripts/mobile/build-ios.sh [--debug]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

profile=release
cargo_profile=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  profile=debug
  cargo_profile=()
fi

# One deployment target for Rust and the C code that cc builds (aws-lc, ring,
# blake3); matches mobile/ios/project.yml. Without it the C objects target the
# SDK's iOS version while Rust links for iOS 10, and `___chkstk_darwin` is missing.
export IPHONEOS_DEPLOYMENT_TARGET=17.0

crate=yttt-mobile-core
lib=libyttt_mobile_core
out="$root/target/mobile/ios"

for target in aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo build -p "$crate" "${cargo_profile[@]}" --target "$target"
done
# Bindings come from the host build: UniFFI reads the same metadata from it.
cargo build -p "$crate" "${cargo_profile[@]}"

rm -rf "$out"
mkdir -p "$out/swift" "$out/headers"
uniffi-bindgen generate --library "target/$profile/$lib.dylib" \
  --language swift --out-dir "$out/swift"
mv "$out/swift/"*.h "$out/headers/"
# Xcode looks for module.modulemap inside an XCFramework's headers.
mv "$out/swift/"*.modulemap "$out/headers/module.modulemap"

xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/$profile/$lib.a" -headers "$out/headers" \
  -library "target/aarch64-apple-ios-sim/$profile/$lib.a" -headers "$out/headers" \
  -output "$out/YtttMobileCore.xcframework" >/dev/null

echo "Built $out/YtttMobileCore.xcframework"
echo "Swift bindings: $out/swift/"
