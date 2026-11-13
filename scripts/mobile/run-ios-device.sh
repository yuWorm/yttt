#!/usr/bin/env bash
# Builds the iOS app for a connected iPhone, installs it and launches it.
#
# Needs the core built first (scripts/mobile/build-ios.sh) and an Apple
# Development certificate in the keychain. The team is $YTTT_IOS_TEAM, or the
# team of the only Apple Development certificate; it stays out of the repo.
# The device is $YTTT_IOS_DEVICE (a devicectl identifier or name), or the
# first available iPhone.
#
# With YTTT_LOG set (a filter, as RUST_LOG), the core logs to stderr and the
# script stays attached to the app to show it.
#
# Usage: [YTTT_LOG=debug] scripts/mobile/run-ios-device.sh [--release] [--no-launch]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root/mobile/ios"

configuration=Debug
launch=1
for arg in "$@"; do
  case "$arg" in
    --release) configuration=Release ;;
    --no-launch) launch=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [[ ! -d "$root/target/mobile/ios/YtttMobileCore.xcframework" ]]; then
  echo "The core is not built; run scripts/mobile/build-ios.sh first." >&2
  exit 1
fi

team="${YTTT_IOS_TEAM:-}"
if [[ -z "$team" ]]; then
  # A certificate's OU is its team.
  teams=$(security find-certificate -a -c "Apple Development" -p 2>/dev/null |
    awk '/BEGIN CERT/{c=""} {c=c $0 "\n"} /END CERT/{printf "%s", c | "openssl x509 -noout -subject"; close("openssl x509 -noout -subject")}' |
    sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p' | sort -u)
  if [[ $(printf '%s\n' "$teams" | grep -c .) -ne 1 ]]; then
    echo "Set YTTT_IOS_TEAM: found Apple Development teams: ${teams:-none}" >&2
    exit 1
  fi
  team="$teams"
fi

devices=$(mktemp)
trap 'rm -f "$devices"' EXIT
xcrun devicectl list devices --json-output "$devices" >/dev/null
device=$(python3 - "$devices" "${YTTT_IOS_DEVICE:-}" <<'EOF'
import json, sys
wanted = sys.argv[2]
for d in json.load(open(sys.argv[1]))["result"]["devices"]:
    name = d["deviceProperties"].get("name", "")
    ident = d["identifier"]
    if wanted:
        if wanted in (ident, name):
            print(ident); break
    # The tunnel opens on demand; a transport means the phone is reachable.
    elif d["hardwareProperties"].get("deviceType") == "iPhone" and d["connectionProperties"].get("transportType") in ("wired", "localNetwork"):
        print(ident); break
EOF
)
if [[ -z "$device" ]]; then
  echo "No connected iPhone; plug it in, unlock it, and trust this Mac." >&2
  exit 1
fi

xcodegen generate >/dev/null
derived="$root/target/ios-device-dd"
xcodebuild build -project Yttt.xcodeproj -scheme Yttt -configuration "$configuration" \
  -destination generic/platform=iOS -derivedDataPath "$derived" \
  -skipPackagePluginValidation -allowProvisioningUpdates \
  DEVELOPMENT_TEAM="$team" -quiet

app="$derived/Build/Products/$configuration-iphoneos/Yttt.app"
xcrun devicectl device install app --device "$device" "$app"
if [[ $launch == 1 ]]; then
  if [[ -n "${YTTT_LOG:-}" ]]; then
    # The core logs to stderr with this filter; stay attached to show it.
    env_json=$(python3 -c 'import json, os; print(json.dumps({"YTTT_LOG": os.environ["YTTT_LOG"]}))')
    xcrun devicectl device process launch --device "$device" --terminate-existing \
      --console --environment-variables "$env_json" com.yttt.yai
  else
    xcrun devicectl device process launch --device "$device" --terminate-existing com.yttt.yai
  fi
fi
