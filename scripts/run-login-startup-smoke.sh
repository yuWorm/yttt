#!/usr/bin/env bash
set -euo pipefail

if [[ "${YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE:-}" != "1" ]]; then
  echo "Refusing to modify login startup outside an explicitly disposable OS user." >&2
  echo "Set YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE=1 only in a disposable VM or OS account." >&2
  exit 2
fi

binary="${1:-}"
if [[ -z "$binary" || ! -x "$binary" ]]; then
  echo "Usage: YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE=1 $0 /path/to/yttt" >&2
  exit 2
fi

profile_id="${YTTT_LOGIN_STARTUP_PROFILE_ID:-default}"
phase="${YTTT_LOGIN_STARTUP_SMOKE_PHASE:-immediate}"
state_file="${YTTT_LOGIN_STARTUP_SMOKE_STATE_FILE:-}"
startup_timeout="${YTTT_LOGIN_STARTUP_SMOKE_TIMEOUT_SECS:-30}"
smoke_config_home="${YTTT_LOGIN_STARTUP_SMOKE_CONFIG_HOME:-}"

case "$phase" in
  immediate|prepare|verify) ;;
  *)
    echo "YTTT_LOGIN_STARTUP_SMOKE_PHASE must be immediate, prepare, or verify." >&2
    exit 2
    ;;
esac
case "$startup_timeout" in
  ''|*[!0-9]*|0)
    echo "YTTT_LOGIN_STARTUP_SMOKE_TIMEOUT_SECS must be a positive integer." >&2
    exit 2
    ;;
esac

source_bundle=""
temporary_bundle_root=""
temporary_bundle=""
smoke_bundle_id=""
original_xdg_config_home="${XDG_CONFIG_HOME-}"
original_xdg_config_home_set="${XDG_CONFIG_HOME+x}"
smoke_agent_label=""
install_macos_smoke_bundle() {
  local destination="$1"
  /usr/bin/ditto "$source_bundle" "$destination"
  /usr/bin/plutil -replace CFBundleIdentifier -string "$smoke_bundle_id" \
    "$destination/Contents/Info.plist"
  /usr/bin/plutil -replace Label -string "$smoke_agent_label" \
    "$destination/Contents/Library/LaunchAgents/com.yttt.host.plist"
  /usr/bin/plutil -replace EnvironmentVariables -json \
    "{\"XDG_CONFIG_HOME\":\"$smoke_config_home\"}" \
    "$destination/Contents/Library/LaunchAgents/com.yttt.host.plist"
  /usr/bin/codesign --force --sign - "$destination" >/dev/null
  /usr/bin/codesign --verify --deep --strict "$destination"
}
prepare_unique_macos_smoke_bundle() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    return
  fi
  case "$binary" in
    *.app/Contents/MacOS/*) ;;
    *)
      echo "macOS smoke requires a packaged executable inside an .app bundle." >&2
      exit 2
      ;;
  esac
  if [[ -z "$smoke_config_home" ]]; then
    echo "macOS smoke requires YTTT_LOGIN_STARTUP_SMOKE_CONFIG_HOME pointing to a disposable, empty directory." >&2
    exit 2
  fi
  if [[ -e "$smoke_config_home" && -n "$(find "$smoke_config_home" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    echo "Refusing to reuse a non-empty macOS smoke config root: $smoke_config_home" >&2
    exit 1
  fi
  mkdir -p "$smoke_config_home"
  export XDG_CONFIG_HOME="$smoke_config_home"
  source_bundle="${binary%/Contents/MacOS/*}"
  if [[ ! -f "$source_bundle/Contents/Library/LaunchAgents/com.yttt.host.plist" ]]; then
    echo "Packaged app is missing the yttt LaunchAgent plist." >&2
    exit 2
  fi
  temporary_bundle_root="$(mktemp -d "${TMPDIR:-/tmp}/yttt-login-startup-smoke.XXXXXX")"
  temporary_bundle="$temporary_bundle_root/yttt-login-startup-smoke.app"
  local identity_suffix="$$-${RANDOM}-${RANDOM}"
  smoke_bundle_id="com.yttt.login-startup-smoke-${identity_suffix}"
  smoke_agent_label="${smoke_bundle_id}.host"
  install_macos_smoke_bundle "$temporary_bundle"
  binary="$temporary_bundle/Contents/MacOS/${binary##*/}"
}
replace_macos_smoke_bundle_for_update() {
  local staged_bundle="$temporary_bundle_root/yttt-login-startup-smoke.update.app"
  local previous_bundle="$temporary_bundle_root/yttt-login-startup-smoke.previous.app"
  install_macos_smoke_bundle "$staged_bundle"
  mv "$temporary_bundle" "$previous_bundle"
  mv "$staged_bundle" "$temporary_bundle"
  rm -rf "$previous_bundle"
}

login_status() {
  "$binary" --login-startup-status --profile-id "$profile_id"
}
host_status() {
  "$binary" --host-status --profile-id "$profile_id"
}
require_inactive_registration() {
  local status="$1"
  case "$status" in
    *disabled*|*unavailable*) ;;
    *)
      echo "Expected no active login startup registration, got: $status" >&2
      return 1
      ;;
  esac
}
require_enabled_registration() {
  local status="$1"
  case "$status" in
    *enabled*|*"requires approval"*) ;;
    *)
      echo "Login startup did not become registered: $status" >&2
      return 1
      ;;
  esac
}
wait_for_host_running() {
  local attempts=$((startup_timeout * 4))
  local attempt
  local status=""
  for ((attempt = 0; attempt < attempts; attempt++)); do
    status="$(host_status 2>&1 || true)"
    case "$status" in
      "Host Running:"*) return 0 ;;
    esac
    sleep 0.25
  done
  echo "Registered startup did not make the Host reachable within ${startup_timeout}s: $status" >&2
  echo "For startup items that run only at login, use prepare before and verify after a real sign-out/sign-in." >&2
  return 1
}
wait_for_host_stopped() {
  local attempts=$((startup_timeout * 4))
  local attempt
  local status=""
  for ((attempt = 0; attempt < attempts; attempt++)); do
    status="$(host_status 2>&1 || true)"
    if [[ "$status" == "Host stopped" ]]; then
      return 0
    fi
    sleep 0.25
  done
  echo "Host did not stop within ${startup_timeout}s: $status" >&2
  return 1
}
stop_host_if_running() {
  if [[ "$(host_status)" != "Host stopped" ]]; then
    "$binary" --stop-host --profile-id "$profile_id" >/dev/null
    wait_for_host_stopped
  fi
}
disable_and_assert() {
  "$binary" --disable-login-startup --profile-id "$profile_id" >/dev/null
  local disabled_status
  disabled_status="$(login_status)"
  require_inactive_registration "$disabled_status"
  printf '%s\n' "$disabled_status"
}

cleanup_state_file=0
cleanup() {
  "$binary" --disable-login-startup --profile-id "$profile_id" >/dev/null 2>&1 || true
  "$binary" --stop-host --profile-id "$profile_id" >/dev/null 2>&1 || true
  if [[ "$cleanup_state_file" -eq 1 && -n "$state_file" ]]; then
    rm -f "$state_file"
  fi
  if [[ -n "$temporary_bundle_root" ]]; then
    rm -rf "$temporary_bundle_root"
    if [[ -n "$smoke_config_home" ]]; then
      rm -rf "$smoke_config_home"
    fi
    if [[ -n "$original_xdg_config_home_set" ]]; then
      export XDG_CONFIG_HOME="$original_xdg_config_home"
    else
      unset XDG_CONFIG_HOME
    fi
  fi
}

case "$phase" in
  immediate)
    prepare_unique_macos_smoke_bundle
    if [[ -n "$temporary_bundle_root" ]]; then
      trap cleanup EXIT
    fi
    initial_login_status="$(login_status)"
    require_inactive_registration "$initial_login_status"
    initial_host_status="$(host_status)"
    if [[ "$initial_host_status" != "Host stopped" ]]; then
      echo "Refusing to mask startup behavior with an already-running Host: $initial_host_status" >&2
      exit 1
    fi
    if [[ -z "$temporary_bundle_root" ]]; then
      trap cleanup EXIT
    fi

    "$binary" --enable-login-startup --confirm-remote-access --profile-id "$profile_id"
    enabled_status="$(login_status)"
    require_enabled_registration "$enabled_status"
    wait_for_host_running
    update_status=""
    if [[ -n "$temporary_bundle_root" ]]; then
      stop_host_if_running
      replace_macos_smoke_bundle_for_update
      "$binary" --enable-login-startup --confirm-remote-access --profile-id "$profile_id" >/dev/null
      update_status="$(login_status)"
      require_enabled_registration "$update_status"
      wait_for_host_running
    fi
    disabled_status="$(disable_and_assert)"
    stop_host_if_running
    if [[ -n "$temporary_bundle_root" ]]; then
      rm -rf "$temporary_bundle_root" "$smoke_config_home"
      temporary_bundle_root=""
      if [[ -n "$original_xdg_config_home_set" ]]; then
        export XDG_CONFIG_HOME="$original_xdg_config_home"
      else
        unset XDG_CONFIG_HOME
      fi
    fi

    trap - EXIT
    if [[ -n "$update_status" ]]; then
      printf 'Login startup smoke passed: %s -> %s -> Host Running -> Host stopped -> update %s -> Host Running -> %s\n' \
        "$initial_login_status" "$enabled_status" "$update_status" "$disabled_status"
    else
      printf 'Login startup smoke passed: %s -> %s -> Host Running -> %s\n' \
        "$initial_login_status" "$enabled_status" "$disabled_status"
    fi
    ;;

  prepare)
    if [[ -z "$state_file" ]]; then
      echo "prepare requires YTTT_LOGIN_STARTUP_SMOKE_STATE_FILE on storage that survives sign-out." >&2
      exit 2
    fi
    if [[ -e "$state_file" ]]; then
      echo "Refusing to overwrite existing smoke state: $state_file" >&2
      exit 1
    fi
    initial_login_status="$(login_status)"
    require_inactive_registration "$initial_login_status"
    if [[ "$(host_status)" != "Host stopped" ]]; then
      echo "prepare requires a stopped Host so the next login proves startup behavior." >&2
      exit 1
    fi
    trap cleanup EXIT

    "$binary" --enable-login-startup --confirm-remote-access --profile-id "$profile_id"
    enabled_status="$(login_status)"
    require_enabled_registration "$enabled_status"
    stop_host_if_running
    (umask 077 && printf '%s\n%s\n' "$binary" "$profile_id" > "$state_file")

    trap - EXIT
    printf 'Login startup prepared. Sign out and back in, then run phase=verify with state file %s\n' \
      "$state_file"
    ;;

  verify)
    if [[ -z "$state_file" || ! -f "$state_file" ]]; then
      echo "verify requires the state file created by phase=prepare." >&2
      exit 2
    fi
    expected_binary="$(sed -n '1p' "$state_file")"
    expected_profile="$(sed -n '2p' "$state_file")"
    if [[ "$expected_binary" != "$binary" || "$expected_profile" != "$profile_id" ]]; then
      echo "Smoke state does not match this binary and profile." >&2
      exit 1
    fi
    cleanup_state_file=1
    trap cleanup EXIT

    enabled_status="$(login_status)"
    require_enabled_registration "$enabled_status"
    wait_for_host_running
    disabled_status="$(disable_and_assert)"
    stop_host_if_running
    rm -f "$state_file"

    trap - EXIT
    printf 'Post-login startup smoke passed: %s -> Host Running -> %s\n' \
      "$enabled_status" "$disabled_status"
    ;;
esac
