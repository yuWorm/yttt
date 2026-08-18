#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/run-terminal-perf.sh [options]

Runs identical release-mode terminal workloads through the standalone direct
backend and the desktop-to-Host backend. Raw JSON is retained for every run;
optional Instruments traces are retained for the first run of each
backend/scenario. The strict summarizer enforces the acceptance thresholds.

Options:
  --backend NAME        direct, host, or both (default: both)
  --scenario NAME       full, damage, scroll, burst, interactive, or all
                        (default: all)
  --runs COUNT          same-machine repetitions per backend/scenario (default: 5)
  --duration SECONDS    measured workload duration after warmup (default: 20)
  --warmup SECONDS      warmup excluded from metrics (default: 3)
  --catch-up SECONDS    final-sentinel/backlog observation window (default: 2)
  --fps FPS             deterministic update rate (default: 60)
  --rows ROWS           terminal rows (default: 40)
  --columns COLUMNS     terminal columns (default: 120)
  --panes COUNT         local Host panes, 1 through 16 (default: 1)
  --clients MODE        one, two, or slow (default: one)
  --lifecycle MODE      open, exited, or reopened (default: open)
  --trace MODE          none, time, or metal (default: time on macOS, none elsewhere)
  --ascii-only          omit CJK and emoji from workload output
  --output-dir PATH     result root (default: tmp/terminal-perf/<timestamp>)
  --no-build            reuse release binaries
  --host-binary PATH    override the Host-backed desktop executable
  --resource-only       run only zero-pane headless Host probes (requires --allow-incomplete)
  --allow-incomplete    permit a partial smoke matrix in the final summary
  -h, --help            show this help

The strict default runs five direct and five Host repetitions for all five
scenarios, then five-run Host coverage cohorts for 4/16 panes, two/slow
clients, exited/reopened desktops, and a zero-pane headless Host resource
probe. It retains raw traces for each baseline scenario and enforces final
sentinel, zero backlog, latency, throughput, CPU, and RSS thresholds. Use
--allow-incomplete only for a targeted development smoke.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command_path() {
  command -v "$1" 2>/dev/null || true
}

activate_process() {
  local pid="$1"
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  /usr/bin/osascript -e \
    "tell application \"System Events\" to set frontmost of first process whose unix id is $pid to true" \
    >/dev/null 2>&1 || true
}
strict_graphical_session_ready() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  local session
  session="$(ioreg -n Root -d1 2>/dev/null || true)"
  [[ "$session" != *'"CGSSessionScreenIsLocked"=Yes'* ]]
}

wait_for_file() {
  local path="$1" pid="$2" timeout_seconds="$3"
  local deadline=$((SECONDS + timeout_seconds))
  while [[ ! -f "$path" ]]; do
    kill -0 "$pid" 2>/dev/null || return 1
    ((SECONDS < deadline)) || return 1
    sleep 0.05
  done
}
wait_for_path() {
  local path="$1" timeout_seconds="$2"
  local deadline=$((SECONDS + timeout_seconds))
  while [[ ! -f "$path" ]]; do
    ((SECONDS < deadline)) || return 1
    sleep 0.05
  done
}


wait_for_json_phase() {
  local path="$1" phase="$2" timeout_seconds="$3"
  local deadline=$((SECONDS + timeout_seconds))
  while ((SECONDS < deadline)); do
    if [[ -f "$path" ]] && python3 - "$path" "$phase" <<'PY'
import json
import sys
try:
    with open(sys.argv[1], encoding="utf-8") as source:
        raise SystemExit(0 if json.load(source).get("phase") == sys.argv[2] else 1)
except (OSError, json.JSONDecodeError):
    raise SystemExit(1)
PY
    then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

terminate_pid() {
  local pid="$1"
  kill -CONT "$pid" 2>/dev/null || true
  kill -TERM "$pid" 2>/dev/null || true
  for _ in {1..50}; do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid" 2>/dev/null || true
  fi
  wait "$pid" 2>/dev/null || true
}

active_desktop_pid=""
active_second_pid=""
active_profile_root=""

cleanup_active_processes() {
  [[ -n "$active_second_pid" ]] && terminate_pid "$active_second_pid"
  [[ -n "$active_desktop_pid" ]] && terminate_pid "$active_desktop_pid"
  if [[ -n "$active_profile_root" && -f "$active_profile_root/runtime/host.pid" ]]; then
    local host_pid
    read -r host_pid <"$active_profile_root/runtime/host.pid" || true
    if [[ "$host_pid" =~ ^[0-9]+$ ]]; then
      terminate_pid "$host_pid"
    fi
  fi
}
trap cleanup_active_processes EXIT

latest_host_session_id() {
  local path="$1"
  python3 - "$path" <<'PY'
import json
import sys
try:
    lines = [line for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
    for line in reversed(lines):
        for terminal in json.loads(line).get("terminals", []):
            session_id = terminal.get("session_id")
            if isinstance(session_id, str) and session_id:
                print(session_id)
                raise SystemExit(0)
except (OSError, json.JSONDecodeError):
    pass
PY
}

rss_bytes() {
  local pid="$1" rss
  rss="$(ps -o rss= -p "$pid" 2>/dev/null | tr -d '[:space:]' || true)"
  if [[ "$rss" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$((rss * 1024))"
  fi
}
physical_footprint_bytes() {
  local pid="$1"
  if [[ "$(uname -s)" != "Darwin" || -z "$(command_path footprint)" ]]; then
    rss_bytes "$pid"
    return
  fi
  local report value=""
  report="$(mktemp "${TMPDIR:-/tmp}/yttt-footprint.XXXXXX")"
  if footprint -j "$report" -p "$pid" >/dev/null 2>&1; then
    value="$(python3 - "$report" <<'PY'
import json
import sys
try:
    processes = json.load(open(sys.argv[1], encoding="utf-8")).get("processes", [])
    if processes and isinstance(processes[0].get("footprint"), int):
        print(processes[0]["footprint"])
except (OSError, json.JSONDecodeError):
    pass
PY
)"
  fi
  rm -f "$report"
  if [[ "$value" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$value"
  else
    rss_bytes "$pid"
  fi
}


latest_host_value() {
  local path="$1" field="$2"
  python3 - "$path" "$field" <<'PY'
import json
import sys
try:
    lines = [line for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
    value = json.loads(lines[-1]).get(sys.argv[2]) if lines else None
    if value is not None:
        print(value)
except (OSError, json.JSONDecodeError):
    pass
PY
}

wait_for_host_data_clients() {
  local path="$1" pid="$2" expected="$3" timeout_seconds="$4"
  local deadline=$((SECONDS + timeout_seconds))
  while ((SECONDS < deadline)); do
    kill -0 "$pid" 2>/dev/null || return 1
    if python3 - "$path" "$expected" <<'PY'
import json
import sys
try:
    lines = [line for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
    snapshot = json.loads(lines[-1]) if lines else {}
    queues = [
        queue
        for queue in snapshot.get("queues", [])
        if queue.get("name") == "attachment_output_bytes"
    ]
    expected = int(sys.argv[2])
    ready = snapshot.get("attachments", 0) >= expected and len(queues) >= expected
    raise SystemExit(0 if ready else 1)
except (OSError, json.JSONDecodeError):
    raise SystemExit(1)
PY
    then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

write_run_metadata() {
  local path="$1" backend="$2" scenario_name="$3" run_number="$4"
  local session_id="$5" process_rss="$6" host_rss="$7" desktop_rss="$8"
  local process_footprint="$9" host_footprint="${10}" desktop_footprint="${11}"
  python3 - "$path" "$backend" "$scenario_name" "$run_number" "$session_id" \
    "$panes" "$clients" "$lifecycle" "$trace_mode" "$process_rss" "$host_rss" \
    "$desktop_rss" "$process_footprint" "$host_footprint" "$desktop_footprint" \
    "$cohort" <<'PY'
import json
import sys
(
    path, backend, scenario, run_number, session_id, panes, clients,
    lifecycle, trace_mode, process_rss, host_rss, desktop_rss,
    process_footprint, host_footprint, desktop_footprint, cohort,
) = sys.argv[1:]
def optional_int(value):
    return int(value) if value else None
document = {
    "schema_version": 2,
    "backend": backend,
    "scenario": scenario,
    "run": int(run_number),
    "cohort": cohort,
    "session_id": session_id,
    "panes": int(panes),
    "clients": clients,
    "desktop_lifecycle": lifecycle,
    "trace": trace_mode,
    "process_rss_bytes": optional_int(process_rss),
    "host_rss_bytes": optional_int(host_rss),
    "desktop_rss_bytes": optional_int(desktop_rss),
    "process_physical_footprint_bytes": optional_int(process_footprint),
    "host_physical_footprint_bytes": optional_int(host_footprint),
    "desktop_physical_footprint_bytes": optional_int(desktop_footprint),
}
with open(path, "w", encoding="utf-8") as output:
    json.dump(document, output, indent=2, allow_nan=False)
    output.write("\n")
PY
}

create_host_project() {
  local project="$1" run_dir="$2" scenario_name="$3" session_suffix="$4"
  local active_seconds=$((warmup + duration))
  mkdir -p "$project/.yttt"
  python3 - "$project/.yttt/layout.toml" "$project" "$workload" "$run_dir/workload.json" \
    "$run_dir/workload.ready" "$run_dir/workload.start" "$scenario_name" "$active_seconds" \
    "$((warmup + 1))" "$fps" "$rows" "$columns" "$catch_up" "$ascii_only" "$panes" \
    "$session_suffix" <<'PY'
import json
import sys
from pathlib import Path
(
    output, project, workload, metrics, ready, start, scenario, active_seconds,
    burst_delay, fps, rows, columns, catch_up, ascii_only, panes, suffix,
) = sys.argv[1:]
panes = int(panes)
args = [
    workload,
    "--scenario", scenario,
    "--duration", active_seconds,
    "--fps", fps,
    "--rows", rows,
    "--columns", columns,
    "--catch-up-seconds", catch_up,
    "--final-sentinel", "YTTT-PERF-FINAL-SENTINEL",
    "--metrics", metrics,
    "--final-sentinel-file", str(Path(metrics).with_name("final-sentinel.ready")),
    "--ready-file", ready,
    "--start-file", start,
]
if scenario == "burst":
    args.extend(["--startup-delay", burst_delay])
if ascii_only == "1":
    args.append("--ascii-only")
def pane(index):
    if index == 0:
        return {
            "type": "pane", "id": "perf", "title": "Performance",
            "command": sys.executable, "args": args,
            "execution_mode": "command", "exit_behavior": "manual_restart",
        }
    return {
        "type": "pane", "id": f"idle-{index}", "title": f"Idle {index}",
        "command": "/bin/sh", "args": ["-lc", f"sleep {float(active_seconds) + float(catch_up) + 1.0}"],
        "execution_mode": "command", "exit_behavior": "manual_restart",
    }
def split(nodes, depth=0):
    if len(nodes) == 1:
        return nodes[0]
    midpoint = len(nodes) // 2
    return {
        "type": "split",
        "direction": "horizontal" if depth % 2 == 0 else "vertical",
        "ratio": 0.5,
        "left": split(nodes[:midpoint], depth + 1),
        "right": split(nodes[midpoint:], depth + 1),
    }
def toml_value(value):
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {toml_value(item)}" for key, item in value.items()) + " }"
    return str(value)

layout = split([pane(index) for index in range(panes)])
text = "\n".join([
    "[project]",
    f"name = {toml_value('Terminal Performance ' + suffix)}",
    'default_tab = "perf"',
    "",
    "[[tabs]]",
    'id = "perf"',
    'title = "Performance"',
    'startup = "eager"',
    f"cwd = {toml_value(project)}",
    f"layout = {toml_value(layout)}",
    "",
])
Path(output).write_text(text, encoding="utf-8")
PY
}
stage_profile_project_layout() {
  local profile_root="$1" project="$2"
  python3 - "$profile_root" "$project" <<'PY'
import shutil
import sys
from pathlib import Path
profile_root = Path(sys.argv[1])
project = Path(sys.argv[2]).resolve()
encoded = "".join(
    chr(byte) if chr(byte).isalnum() and byte < 128 or byte in b"-_."
    else f"%{byte:02x}"
    for byte in str(project).encode()
)
destination = profile_root / "state" / "project-config-overlay" / encoded / "layout.toml"
destination.parent.mkdir(parents=True, exist_ok=True)
shutil.copyfile(project / ".yttt" / "layout.toml", destination)
PY
}
stage_profile_recent_project() {
  local profile_root="$1" project="$2"
  python3 - "$profile_root" "$project" <<'PY'
import sys
import uuid
from pathlib import Path
profile_root = Path(sys.argv[1])
project = Path(sys.argv[2]).resolve()
namespace = uuid.UUID("d9e388e6-0a25-49e2-b0ae-9f6567e42131")
project_id = str(uuid.uuid5(namespace, str(project)))
quoted_path = str(project).replace("\\", "\\\\").replace('"', '\\"')
config = profile_root / "config" / "recent-projects.toml"
config.write_text(
    "\n".join([
        "version = 1",
        "",
        "[[projects]]",
        f'id = "{project_id}"',
        'title = "Terminal Performance"',
        'kind = "local"',
        f'path = "{quoted_path}"',
        "",
        "[[last_opened_projects]]",
        f'id = "{project_id}"',
        'kind = "local"',
        f'path = "{quoted_path}"',
        "",
        "[[last_restorable_projects]]",
        f'id = "{project_id}"',
        'kind = "local"',
        f'path = "{quoted_path}"',
        "",
    ]),
    encoding="utf-8",
)
PY
}

run_trace_and_open_gate() {
  local pid="$1" gate="$2" run_dir="$3" label="$4" run_number="$5"
  if [[ "$cohort" != "baseline" || "$trace_mode" == "none" || "$run_number" != "1" ]]; then
    : >"$gate"
    return 0
  fi
  local template trace_path trace_log trace_duration trace_pid
  if [[ "$trace_mode" == "time" ]]; then
    template="Time Profiler"
  else
    template="Metal System Trace"
  fi
  trace_path="$run_dir/$label-$trace_mode.trace"
  trace_log="$run_dir/xctrace.log"
  trace_duration=$((warmup + duration + catch_up + 5))
  xcrun xctrace record \
    --template "$template" \
    --output "$trace_path" \
    --time-limit "${trace_duration}s" \
    --no-prompt \
    --attach "$pid" >"$trace_log" 2>&1 &
  trace_pid=$!
  local deadline=$((SECONDS + 15))
  while ((SECONDS < deadline)); do
    if [[ -f "$trace_log" ]] && grep -q "Ctrl-C to stop" "$trace_log"; then
      activate_process "$pid"
      sleep 1
      : >"$gate"
      wait "$trace_pid" || [[ -d "$trace_path" ]]
      return 0
    fi
    kill -0 "$trace_pid" 2>/dev/null || break
    sleep 0.1
  done
  terminate_pid "$trace_pid"
  return 1
}

export_trace() {
  local run_dir="$1" label="$2" trace_path
  trace_path="$run_dir/$label-$trace_mode.trace"
  [[ "$trace_mode" != "none" && -d "$trace_path" ]] || return 0
  xcrun xctrace export --input "$trace_path" --toc --output "$run_dir/toc.xml"
  xcrun xctrace export \
    --input "$trace_path" \
    --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' \
    --output "$run_dir/time-profile.xml" || true
  if [[ "$trace_mode" == "metal" ]]; then
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="ca-client-present-request"]' \
      --output "$run_dir/present-requests.xml" || true
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="displayed-surfaces-interval"]' \
      --output "$run_dir/displayed-surfaces.xml" || true
  fi
}

common_workload_command() {
  local scenario_name="$1" run_dir="$2"
  WORKLOAD_COMMAND=(
    python3 "$workload"
    --scenario "$scenario_name"
    --duration "$((warmup + duration))"
    --fps "$fps"
    --rows "$rows"
    --columns "$columns"
    --catch-up-seconds "$catch_up"
    --final-sentinel "YTTT-PERF-FINAL-SENTINEL"
    --metrics "$run_dir/workload.json"
    --final-sentinel-file "$run_dir/final-sentinel.ready"
    --ready-file "$run_dir/workload.ready"
    --start-file "$run_dir/workload.start"
  )
  if [[ "$scenario_name" == "burst" ]]; then
    WORKLOAD_COMMAND+=(--startup-delay "$((warmup + 1))")
  fi
  if ((ascii_only)); then
    WORKLOAD_COMMAND+=(--ascii-only)
  fi
}

run_direct() {
  local scenario_name="$1" run_number="$2"
  local run_dir="$output_dir/$cohort/direct/$scenario_name/run-$(printf '%02d' "$run_number")"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  common_workload_command "$scenario_name" "$run_dir"
  local workload_shell
  printf -v workload_shell '%q ' "${WORKLOAD_COMMAND[@]}"
  local -a performance_env=(
    "SHELL=/bin/bash"
    "YTTT_TERMINAL_START_COMMAND=$workload_shell"
    "YTTT_TERMINAL_PERF_OUTPUT=$run_dir/metrics.json"
    "YTTT_TERMINAL_PERF_LABEL=direct"
    "YTTT_TERMINAL_PERF_SCENARIO=$scenario_name"
    "YTTT_TERMINAL_PERF_WARMUP_SECONDS=$warmup"
    "YTTT_TERMINAL_PERF_DURATION_SECONDS=$((duration + catch_up + 1))"
    "YTTT_TERMINAL_PERF_START_FILE=$run_dir/workload.start"
    "YTTT_TERMINAL_PERF_READY_FILE=$run_dir/terminal.ready"
    "YTTT_TERMINAL_PERF_FINAL_SENTINEL=YTTT-PERF-FINAL-SENTINEL"
    "YTTT_TERMINAL_PERF_REPORT_INTERVAL_MS=100"
  )
  if [[ "$scenario_name" == "interactive" ]]; then
    performance_env+=(
      "YTTT_TERMINAL_PERF_INPUT_DELAY_MS=$((warmup * 1000 + 1000))"
      "YTTT_TERMINAL_PERF_INPUT_SAMPLES=620"
      "YTTT_TERMINAL_PERF_INPUT_INTERVAL_MS=10"
      "YTTT_TERMINAL_PERF_PREEDIT_INTERVAL_MS=5"
    )
  fi
  printf 'run: backend=direct scenario=%s run=%s output=%s\n' \
    "$scenario_name" "$run_number" "$run_dir"
  env "${performance_env[@]}" "$direct_binary" &
  local pid=$!
  active_desktop_pid="$pid"
  if ! wait_for_file "$run_dir/terminal.ready" "$pid" 20; then
    terminate_pid "$pid"
    die "direct terminal did not initialize its reporter"
  fi
  activate_process "$pid"
  if ! wait_for_file "$run_dir/workload.ready" "$pid" 20; then
    terminate_pid "$pid"
    die "direct workload did not reach its start gate"
  fi
  local process_rss="" process_footprint=""
  sleep 2
  process_rss="$(rss_bytes "$pid")"
  process_footprint="$(physical_footprint_bytes "$pid")"
  run_trace_and_open_gate "$pid" "$run_dir/workload.start" "$run_dir" direct "$run_number" || {
    terminate_pid "$pid"
    die "direct trace failed"
  }
  local timeout_seconds=$((warmup + duration + catch_up + 20))
  wait_for_file "$run_dir/workload.json" "$pid" "$timeout_seconds" || {
    terminate_pid "$pid"
    die "direct workload did not finish"
  }
  wait_for_json_phase "$run_dir/metrics.json" finished "$timeout_seconds" || {
    terminate_pid "$pid"
    die "direct performance report did not finish"
  }
  terminate_pid "$pid"
  active_desktop_pid=""
  export_trace "$run_dir" direct
  write_run_metadata "$run_dir/run.json" direct "$scenario_name" "$run_number" \
    "direct:$scenario_name" "$process_rss" "" "" "$process_footprint" "" ""
}

run_host() {
  local scenario_name="$1" run_number="$2"
  local run_dir="$output_dir/$cohort/host/$scenario_name/run-$(printf '%02d' "$run_number")"
  local profile_root project diagnostics_path session_id
  profile_root="$(mktemp -d "${TMPDIR:-/tmp}/yttt-perf.XXXXXX")"
  active_profile_root="$profile_root"
  project="$run_dir/project"
  diagnostics_path="$profile_root/runtime/host-diagnostics.jsonl"
  session_id=""
  rm -rf "$run_dir"
  mkdir -p "$profile_root/config"
  printf '[general]\nonboarding_completed = true\n\n[window]\neffect = "none"\nopacity = 1.0\n' >"$profile_root/config/settings.toml"
  mkdir -p "$run_dir" "$project"
  create_host_project "$project" "$run_dir" "$scenario_name" "$scenario_name-$run_number"
  stage_profile_project_layout "$profile_root" "$project"
  stage_profile_recent_project "$profile_root" "$project"
  local -a performance_env=(
    "YTTT_PROFILE_ROOT=$profile_root"
    "YTTT_TERMINAL_PERF_OUTPUT=$run_dir/metrics.json"
    "YTTT_TERMINAL_PERF_LABEL=host"
    "YTTT_TERMINAL_PERF_SCENARIO=$scenario_name"
    "YTTT_TERMINAL_PERF_PANE_ID=perf"
    "YTTT_TERMINAL_PERF_WARMUP_SECONDS=$warmup"
    "YTTT_TERMINAL_PERF_DURATION_SECONDS=$((duration + catch_up + 1))"
    "YTTT_TERMINAL_PERF_START_FILE=$run_dir/workload.start"
    "YTTT_TERMINAL_PERF_READY_FILE=$run_dir/terminal.ready"
    "YTTT_TERMINAL_PERF_FINAL_SENTINEL=YTTT-PERF-FINAL-SENTINEL"
    "YTTT_TERMINAL_PERF_REPORT_INTERVAL_MS=100"
  )
  if [[ "$scenario_name" == "interactive" ]]; then
    performance_env+=(
      "YTTT_TERMINAL_PERF_INPUT_DELAY_MS=$((warmup * 1000 + 1000))"
      "YTTT_TERMINAL_PERF_INPUT_SAMPLES=620"
      "YTTT_TERMINAL_PERF_INPUT_INTERVAL_MS=10"
      "YTTT_TERMINAL_PERF_PREEDIT_INTERVAL_MS=5"
    )
  fi
  printf 'run: backend=host scenario=%s run=%s panes=%s clients=%s lifecycle=%s output=%s\n' \
    "$scenario_name" "$run_number" "$panes" "$clients" "$lifecycle" "$run_dir"
  env "${performance_env[@]}" "$host_binary" --project "$project" &
  local desktop_pid=$! second_pid=""
  active_desktop_pid="$desktop_pid"
  if ! wait_for_file "$run_dir/terminal.ready" "$desktop_pid" 30; then
    terminate_pid "$desktop_pid"
    die "Host-backed desktop did not initialize its reporter"
  fi
  activate_process "$desktop_pid"
  if ! wait_for_file "$run_dir/workload.ready" "$desktop_pid" 30; then
    terminate_pid "$desktop_pid"
    die "Host workload did not reach its start gate"
  fi
  if [[ "$clients" != "one" ]]; then
    env YTTT_PROFILE_ROOT="$profile_root" \
      YTTT_TERMINAL_PERF_OUTPUT="$run_dir/second-metrics.json" \
      YTTT_TERMINAL_PERF_PANE_ID=observer \
      "$host_binary" --project "$project" >"$run_dir/second-client.log" 2>&1 &
    second_pid=$!
    active_second_pid="$second_pid"
    wait_for_host_data_clients "$diagnostics_path" "$second_pid" 2 30 || {
      terminate_pid "$second_pid"
      terminate_pid "$desktop_pid"
      die "second desktop did not establish its terminal data attachment"
    }
    if [[ "$clients" == "slow" ]]; then
      kill -STOP "$second_pid"
    fi
  fi
  local desktop_rss="" host_rss="" desktop_footprint="" host_footprint="" host_pid=""
  sleep 2
  desktop_rss="$(rss_bytes "$desktop_pid")"
  desktop_footprint="$(physical_footprint_bytes "$desktop_pid")"
  if [[ -f "$profile_root/runtime/host.pid" ]]; then
    read -r host_pid <"$profile_root/runtime/host.pid" || true
  fi
  host_rss="$(latest_host_value "$diagnostics_path" rss_bytes)"
  if [[ "$host_pid" =~ ^[0-9]+$ ]]; then
    host_footprint="$(physical_footprint_bytes "$host_pid")"
  fi
  run_trace_and_open_gate "$desktop_pid" "$run_dir/workload.start" "$run_dir" host "$run_number" || {
    [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
    terminate_pid "$desktop_pid"
    die "Host trace failed"
  }
  if [[ "$lifecycle" == "reopened" || "$lifecycle" == "exited" ]]; then
    sleep "$((warmup + 1))"
    terminate_pid "$desktop_pid"
    active_desktop_pid=""
    rm -f "$run_dir/terminal.ready" "$run_dir/metrics.json"
    if [[ "$lifecycle" == "exited" ]]; then
      wait_for_path "$run_dir/final-sentinel.ready" "$((duration + catch_up + 20))" || {
        [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
        die "Host workload did not reach its final sentinel while desktop was exited"
      }
      env "${performance_env[@]}" \
        YTTT_TERMINAL_PERF_WARMUP_SECONDS=0 \
        YTTT_TERMINAL_PERF_DURATION_SECONDS="$catch_up" \
        "$host_binary" --project "$project" &
    else
      env "${performance_env[@]}" "$host_binary" --project "$project" &
    fi
    desktop_pid=$!
    active_desktop_pid="$desktop_pid"
    wait_for_file "$run_dir/terminal.ready" "$desktop_pid" 30 || {
      [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
      terminate_pid "$desktop_pid"
      die "reopened desktop did not reattach"
    }
  fi
  activate_process "$desktop_pid"
  local timeout_seconds=$((warmup + duration + catch_up + 30))
  wait_for_file "$run_dir/workload.json" "$desktop_pid" "$timeout_seconds" || {
    [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
    terminate_pid "$desktop_pid"
    die "Host workload did not finish"
  }
  wait_for_json_phase "$run_dir/metrics.json" finished "$timeout_seconds" || {
    [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
    terminate_pid "$desktop_pid"
    die "Host performance report did not finish"
  }
  [[ -n "$second_pid" ]] && terminate_pid "$second_pid"
  active_second_pid=""
  terminate_pid "$desktop_pid"
  active_desktop_pid=""
  sleep 2
  env YTTT_PROFILE_ROOT="$profile_root" "$host_binary" --stop-host || \
    die "Host did not accept DrainAndStop"
  sleep 1
  if [[ -f "$diagnostics_path" ]]; then
    cp "$diagnostics_path" "$run_dir/host-diagnostics.jsonl"
  fi
  if [[ -f "$profile_root/logs/host.log" ]]; then
    cp "$profile_root/logs/host.log" "$run_dir/host.log"
  fi
  session_id="$(latest_host_session_id "$diagnostics_path")"
  if [[ -z "$host_pid" && -f "$profile_root/runtime/host.pid" ]]; then
    read -r host_pid <"$profile_root/runtime/host.pid" || true
  fi
  export_trace "$run_dir" host
  write_run_metadata "$run_dir/run.json" host "$scenario_name" "$run_number" \
    "$session_id" "" "$host_rss" "$desktop_rss" "" "$host_footprint" "$desktop_footprint"
  if [[ "$host_pid" =~ ^[0-9]+$ ]]; then
    terminate_pid "$host_pid"
  fi
  active_profile_root=""
  rm -rf "$profile_root"
}

run_host_idle_probe() {
  local run_number="$1"
  local run_dir="$output_dir/resources/host-only/run-$(printf '%02d' "$run_number")"
  local profile_root diagnostics_path auth_token host_pid host_footprint
  profile_root="$(mktemp -d "${TMPDIR:-/tmp}/yttt-perf-idle.XXXXXX")"
  active_profile_root="$profile_root"
  diagnostics_path="$profile_root/runtime/host-diagnostics.jsonl"
  auth_token="$profile_root/runtime/host-auth-token"
  rm -rf "$run_dir"
  mkdir -p "$run_dir" "$profile_root/runtime" "$profile_root/config" "$profile_root/logs"
  chmod 700 "$profile_root/runtime"
  python3 - "$auth_token" <<'PY'
import os
import secrets
import sys
path = sys.argv[1]
descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
with os.fdopen(descriptor, "wb") as output:
    output.write(secrets.token_bytes(32))
PY
  printf 'run: backend=host-only panes=0 run=%s output=%s\n' "$run_number" "$run_dir"
  "$host_binary" \
    --process-role=host \
    --profile-id performance \
    --runtime-root "$profile_root/runtime" \
    --auth-token-file "$auth_token" \
    --ssh-host-keys-file "$profile_root/config/ssh-host-keys.toml" \
    --credential-namespace dev.yttt.ssh.performance \
    --build-id "$build_id" \
    >"$run_dir/host.log" 2>&1 &
  host_pid=$!
  wait_for_path "$profile_root/runtime/host-ready.json" 20 || {
    terminate_pid "$host_pid"
    die "headless Host resource probe did not become ready"
  }
  sleep 7
  [[ -f "$diagnostics_path" ]] || {
    terminate_pid "$host_pid"
    die "headless Host resource probe did not emit diagnostics"
  }
  cp "$diagnostics_path" "$run_dir/host-diagnostics.jsonl"
  host_footprint="$(physical_footprint_bytes "$host_pid")"
  python3 - "$run_dir/host-diagnostics.jsonl" "$run_dir/run.json" "$run_number" "$host_footprint" <<'PY'
import json
import math
import statistics
import sys
from pathlib import Path
diagnostics = Path(sys.argv[1])
snapshots = [
    json.loads(line)
    for line in diagnostics.read_text(encoding="utf-8").splitlines()
    if line
]
idle = [
    snapshot
    for snapshot in snapshots
    if snapshot.get("sessions") == 0 and snapshot.get("attachments") == 0
]
cpu = [
    float(snapshot["idle_cpu_percent"])
    for snapshot in idle[1:]
    if isinstance(snapshot.get("idle_cpu_percent"), (int, float))
    and math.isfinite(snapshot["idle_cpu_percent"])
]
rss = [
    int(snapshot["rss_bytes"])
    for snapshot in idle
    if isinstance(snapshot.get("rss_bytes"), int)
]
document = {
    "schema_version": 1,
    "backend": "host-only",
    "cohort": "host-only",
    "run": int(sys.argv[3]),
    "panes": 0,
    "clients": "none",
    "desktop_lifecycle": "exited",
    "samples": len(idle),
    "idle_cpu_percent": statistics.median(cpu) if cpu else None,
    "rss_bytes": int(round(statistics.median(rss))) if rss else None,
    "physical_footprint_bytes": int(sys.argv[4]) if sys.argv[4] else None,
    "thread_count": idle[-1].get("thread_count") if idle else None,
}
Path(sys.argv[2]).write_text(
    json.dumps(document, indent=2, allow_nan=False) + "\n",
    encoding="utf-8",
)
PY
  env YTTT_PROFILE_ROOT="$profile_root" "$host_binary" --stop-host ||
    die "headless Host resource probe did not accept DrainAndStop"
  rm -rf "$profile_root" || true
  active_profile_root=""
}

write_resources() {
  python3 - "$output_dir" <<'PY'
import json
import math
import statistics
import sys
from pathlib import Path
root = Path(sys.argv[1])
def median(values):
    return statistics.median(values) if values else None
def integer_median(values):
    value = median(values)
    return int(round(value)) if value is not None else None
direct_rss = []
direct_footprint = []
combined_rss = []
combined_footprint = []
host_only_rss = []
host_only_footprint = []
host_only_cpu = []
for metadata_path in root.rglob("run.json"):
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    if metadata.get("backend") == "host-only":
        rss = metadata.get("rss_bytes")
        cpu = metadata.get("idle_cpu_percent")
        if isinstance(rss, (int, float)) and math.isfinite(rss):
            host_only_rss.append(rss)
        footprint = metadata.get("physical_footprint_bytes")
        if isinstance(footprint, (int, float)) and math.isfinite(footprint):
            host_only_footprint.append(footprint)
        if isinstance(cpu, (int, float)) and math.isfinite(cpu):
            host_only_cpu.append(cpu)
        continue
    if metadata.get("cohort") != "baseline":
        continue
    if metadata.get("panes") != 1 or metadata.get("clients") != "one":
        continue
    if metadata.get("backend") == "direct":
        if isinstance(metadata.get("process_rss_bytes"), int):
            direct_rss.append(metadata["process_rss_bytes"])
        if isinstance(metadata.get("process_physical_footprint_bytes"), int):
            direct_footprint.append(metadata["process_physical_footprint_bytes"])
        continue
    if metadata.get("backend") != "host":
        continue
    host_rss = metadata.get("host_rss_bytes")
    desktop_rss = metadata.get("desktop_rss_bytes")
    if isinstance(host_rss, int) and isinstance(desktop_rss, int):
        combined_rss.append(host_rss + desktop_rss)
    host_footprint = metadata.get("host_physical_footprint_bytes")
    desktop_footprint = metadata.get("desktop_physical_footprint_bytes")
    if isinstance(host_footprint, int) and isinstance(desktop_footprint, int):
        combined_footprint.append(host_footprint + desktop_footprint)
document = {
    "schema_version": 2,
    "acceptance_memory_metric": (
        "physical_footprint_bytes"
        if direct_footprint and combined_footprint
        else "rss_bytes"
    ),
    "direct_one_pane": {
        "rss_bytes": integer_median(direct_rss),
        "physical_footprint_bytes": integer_median(direct_footprint),
    },
    "host_only": {
        "rss_bytes": integer_median(host_only_rss),
        "physical_footprint_bytes": integer_median(host_only_footprint),
        "idle_cpu_percent": median(host_only_cpu),
    },
    "combined_one_pane": {
        "rss_bytes": integer_median(combined_rss),
        "physical_footprint_bytes": integer_median(combined_footprint),
    },
}
(root / "resources.json").write_text(json.dumps(document, indent=2, allow_nan=False) + "\n", encoding="utf-8")
PY
}

backend="both"
scenario="all"
runs=5
host_binary_override=""
duration=20
warmup=3
catch_up=2
fps=60
rows=40
columns=120
panes=1
clients="one"
lifecycle="open"
trace_mode=""
ascii_only=0
output_dir=""
build=1
allow_incomplete=0
cohort="baseline"
resource_only=0

while (($#)); do
  case "$1" in
    --backend) (($# >= 2)) || die "--backend requires a value"; backend="$2"; shift 2 ;;
    --scenario) (($# >= 2)) || die "--scenario requires a value"; scenario="$2"; shift 2 ;;
    --runs) (($# >= 2)) || die "--runs requires a value"; runs="$2"; shift 2 ;;
    --host-binary) (($# >= 2)) || die "--host-binary requires a value"; host_binary_override="$2"; shift 2 ;;
    --duration) (($# >= 2)) || die "--duration requires a value"; duration="$2"; shift 2 ;;
    --warmup) (($# >= 2)) || die "--warmup requires a value"; warmup="$2"; shift 2 ;;
    --catch-up) (($# >= 2)) || die "--catch-up requires a value"; catch_up="$2"; shift 2 ;;
    --fps) (($# >= 2)) || die "--fps requires a value"; fps="$2"; shift 2 ;;
    --rows) (($# >= 2)) || die "--rows requires a value"; rows="$2"; shift 2 ;;
    --columns) (($# >= 2)) || die "--columns requires a value"; columns="$2"; shift 2 ;;
    --panes) (($# >= 2)) || die "--panes requires a value"; panes="$2"; shift 2 ;;
    --clients) (($# >= 2)) || die "--clients requires a value"; clients="$2"; shift 2 ;;
    --lifecycle) (($# >= 2)) || die "--lifecycle requires a value"; lifecycle="$2"; shift 2 ;;
    --trace) (($# >= 2)) || die "--trace requires a value"; trace_mode="$2"; shift 2 ;;
    --ascii-only) ascii_only=1; shift ;;
    --output-dir) (($# >= 2)) || die "--output-dir requires a value"; output_dir="$2"; shift 2 ;;
    --no-build) build=0; shift ;;
    --allow-incomplete) allow_incomplete=1; shift ;;
    --resource-only) resource_only=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[[ "$backend" =~ ^(direct|host|both)$ ]] || die "--backend must be direct, host, or both"
[[ "$scenario" =~ ^(full|damage|scroll|burst|interactive|all)$ ]] || \
  die "--scenario must be full, damage, scroll, burst, interactive, or all"
[[ "$clients" =~ ^(one|two|slow)$ ]] || die "--clients must be one, two, or slow"
[[ "$lifecycle" =~ ^(open|exited|reopened)$ ]] || \
  die "--lifecycle must be open, exited, or reopened"
for value_name in runs duration warmup catch_up fps rows columns panes; do
  value="${!value_name}"
  [[ "$value" =~ ^[0-9]+$ ]] || die "--${value_name//_/-} must be a non-negative integer"
done
((runs > 0)) || die "--runs must be positive"
((duration > 0)) || die "--duration must be positive"
((catch_up >= 2 || allow_incomplete)) || die "strict runs require --catch-up >= 2"
((fps > 0)) || die "--fps must be positive"
((rows >= 4)) || die "--rows must be at least 4"
((columns >= 20)) || die "--columns must be at least 20"
((panes >= 1 && panes <= 16)) || die "--panes must be between 1 and 16"
if ((allow_incomplete == 0 && runs < 5)); then
  die "strict runs require at least 5 repetitions"
fi
if ((resource_only && allow_incomplete == 0)); then
  die "--resource-only requires --allow-incomplete"
fi
if ((allow_incomplete == 0)) && [[ "$backend" != "both" || "$scenario" != "all" || "$panes" != "1" || "$clients" != "one" || "$lifecycle" != "open" ]]; then
  die "strict acceptance requires --backend both --scenario all --panes 1 --clients one --lifecycle open"
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_id="$(python3 - "$repo_root/Cargo.toml" <<'PY'
import sys
import tomllib
with open(sys.argv[1], "rb") as source:
    manifest = tomllib.load(source)
print(manifest["workspace"]["package"]["version"])
PY
)"
workload="$repo_root/scripts/terminal-perf-workload.py"
[[ -x "$workload" ]] || die "workload is not executable: $workload"
if [[ -z "$trace_mode" ]]; then
  if [[ "$(uname -s)" == "Darwin" ]]; then
    trace_mode="time"
  else
    trace_mode="none"
  fi
fi
[[ "$trace_mode" =~ ^(none|time|metal)$ ]] || die "--trace must be none, time, or metal"
if [[ "$trace_mode" != "none" ]]; then
  [[ "$(uname -s)" == "Darwin" ]] || die "Instruments tracing requires macOS"
  [[ -n "$(command_path xcrun)" ]] || die "xcrun is required for Instruments tracing"
fi
if ((allow_incomplete == 0)) && [[ "$trace_mode" == "none" ]]; then
  die "strict acceptance requires a raw trace"
fi
if [[ "$clients" == "slow" && "$(uname -s)" == "MINGW"* ]]; then
  die "the intentionally slow client mode requires POSIX process suspension"
fi
if ((allow_incomplete == 0)) && ! strict_graphical_session_ready; then
  die "strict acceptance requires an unlocked macOS graphical session"
fi

if [[ -z "$output_dir" ]]; then
  output_dir="$repo_root/tmp/terminal-perf/$(date -u +%Y%m%dT%H%M%SZ)"
elif [[ "$output_dir" != /* ]]; then
  output_dir="$repo_root/$output_dir"
fi
mkdir -p "$output_dir"
if ((build)); then
  if [[ "$backend" == "direct" || "$backend" == "both" ]]; then
    cargo build -p yttt-terminal --release --features perf-metrics
  fi
  if [[ "$backend" == "host" || "$backend" == "both" ]]; then
    cargo build -p yttt --release --features perf-metrics
  fi
fi

target_root="${CARGO_TARGET_DIR:-$repo_root/target}"
[[ "$target_root" == /* ]] || target_root="$repo_root/$target_root"
direct_binary="$target_root/release/yttt-terminal"
host_binary="${host_binary_override:-$target_root/release/yttt}"
if [[ "$host_binary" != /* ]]; then
  host_binary="$repo_root/$host_binary"
fi
if [[ "$backend" == "direct" || "$backend" == "both" ]]; then
  [[ -x "$direct_binary" ]] || die "direct binary not found: $direct_binary"
fi
if [[ "$backend" == "host" || "$backend" == "both" ]]; then
  [[ -x "$host_binary" ]] || die "Host desktop binary not found: $host_binary"
fi

if ((resource_only)); then
  cohort="host-only"
  for run_number in $(seq 1 "$runs"); do
    run_host_idle_probe "$run_number"
  done
  write_resources
  printf '\nresults: %s\n' "$output_dir"
  exit 0
fi
if [[ "$scenario" == "all" ]]; then
  scenarios=(full damage scroll burst interactive)
else
  scenarios=("$scenario")
fi
if [[ "$backend" == "both" ]]; then
  backends=(direct host)
else
  backends=("$backend")
fi

for selected_backend in "${backends[@]}"; do
  for selected_scenario in "${scenarios[@]}"; do
    for run_number in $(seq 1 "$runs"); do
      if [[ "$selected_backend" == "direct" ]]; then
        run_direct "$selected_scenario" "$run_number"
      else
        run_host "$selected_scenario" "$run_number"
      fi
    done
  done
done

if ((allow_incomplete == 0)); then
  coverage_cohorts=(
    "panes-4:4:one:open"
    "panes-16:16:one:open"
    "clients-two:1:two:open"
    "clients-slow:1:slow:open"
    "lifecycle-exited:1:one:exited"
    "lifecycle-reopened:1:one:reopened"
  )
  for coverage in "${coverage_cohorts[@]}"; do
    IFS=: read -r cohort panes clients lifecycle <<<"$coverage"
    for run_number in $(seq 1 "$runs"); do
      run_host damage "$run_number"
    done
  done
  cohort="host-only"
  for run_number in $(seq 1 "$runs"); do
    run_host_idle_probe "$run_number"
  done
fi

write_resources
printf '\nresults: %s\n' "$output_dir"
summary_args=("$output_dir")
if ((allow_incomplete)); then
  summary_args+=(--allow-incomplete)
fi
"$repo_root/scripts/summarize-terminal-perf.py" "${summary_args[@]}"
