#!/usr/bin/env bash
# Builds yttt-mobile-core for Android arm64 and generates the Kotlin bindings
# the Android app compiles.
#
# Output (under target/, not tracked):
#   target/mobile/android/jniLibs/arm64-v8a/libyttt_mobile_core.so
#   target/mobile/android/kotlin/
#
# Needs ANDROID_NDK_HOME (or an NDK under ANDROID_HOME/ndk) and cargo-ndk.
# Usage: scripts/mobile/build-android.sh [--debug]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

profile=release
cargo_profile=(--release)
if [[ "${1:-}" == "--debug" ]]; then
  profile=debug
  cargo_profile=()
fi

if [[ -z "${ANDROID_NDK_HOME:-}" && -n "${ANDROID_HOME:-}" ]]; then
  ANDROID_NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
  export ANDROID_NDK_HOME
fi
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME or ANDROID_HOME}"

out="$root/target/mobile/android"
rm -rf "$out"
mkdir -p "$out/jniLibs" "$out/kotlin"

# minSdk 26, matching mobile/android/app/build.gradle.kts.
cargo ndk -t arm64-v8a --platform 26 build -p yttt-mobile-core "${cargo_profile[@]}"
# Only this library: `cargo ndk -o` would copy every .so in the target
# directory, including dependencies' own cdylibs.
mkdir -p "$out/jniLibs/arm64-v8a"
cp "target/aarch64-linux-android/$profile/libyttt_mobile_core.so" "$out/jniLibs/arm64-v8a/"
uniffi-bindgen generate \
  --library "target/aarch64-linux-android/$profile/libyttt_mobile_core.so" \
  --language kotlin --out-dir "$out/kotlin" --no-format

echo "Built $out/jniLibs/arm64-v8a/libyttt_mobile_core.so"
echo "Kotlin bindings: $out/kotlin/"
