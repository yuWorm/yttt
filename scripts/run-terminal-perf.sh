#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/run-terminal-perf.sh [options]

Runs the same deterministic workload in yttt-terminal, Kitty, or Alacritty.
Optionally records Time Profiler and Metal System Trace captures with xctrace.

Options:
  --terminal NAME       yttt, kitty, alacritty, or all (default: yttt)
  --trace MODE          none, time, metal, or both (default: both)
  --scenario NAME       full, damage, or scroll (default: full)
  --duration SECONDS    measured duration after warmup (default: 20)
  --warmup SECONDS      warmup duration excluded from yttt metrics (default: 3)
  --fps FPS             workload update rate (default: 60)
  --rows ROWS           deterministic payload rows (default: 40)
  --columns COLUMNS     deterministic payload columns (default: 120)
  --ascii-only          omit CJK and emoji from workload output
  --external-only       disable yttt probes for fair cross-terminal tracing

  --output-dir PATH     result root (default: tmp/terminal-perf/<timestamp>)
  --no-build            reuse target/release/yttt-terminal
  -h, --help            show this help

Examples:
  scripts/run-terminal-perf.sh --terminal yttt --trace both --duration 30
  scripts/run-terminal-perf.sh --terminal all --trace metal --scenario damage
  scripts/run-terminal-perf.sh --terminal kitty --trace none --duration 10
  scripts/run-terminal-perf.sh --terminal all --trace time --external-only


During a run, type English or use a Chinese IME in the benchmark window. The
workload echoes committed text. yttt's perf-metrics build records GPUI input to
PTY write, exact echoed-byte parser match, first paint, and IME-preedit paint
latencies. Actual drawable presentation remains in the Metal trace.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command_path() {
  command -v "$1" 2>/dev/null || true
}

wait_for_ready_file() {
  local path="$1"
  local pid="$2"
  local deadline=$((SECONDS + 15))
  while [[ ! -f "$path" ]]; do
    kill -0 "$pid" 2>/dev/null || return 1
    ((SECONDS < deadline)) || return 1
    sleep 0.05
  done
}


terminal_name="yttt"
trace_mode="both"
scenario="full"
duration=20
warmup=3
fps=60
rows=40
columns=120
ascii_only=0
output_dir=""
build=1
internal_metrics_enabled=1


while (($#)); do
  case "$1" in
    --terminal)
      (($# >= 2)) || die "--terminal requires a value"
      terminal_name="$2"
      shift 2
      ;;
    --trace)
      (($# >= 2)) || die "--trace requires a value"
      trace_mode="$2"
      shift 2
      ;;
    --scenario)
      (($# >= 2)) || die "--scenario requires a value"
      scenario="$2"
      shift 2
      ;;
    --duration)
      (($# >= 2)) || die "--duration requires a value"
      duration="$2"
      shift 2
      ;;
    --warmup)
      (($# >= 2)) || die "--warmup requires a value"
      warmup="$2"
      shift 2
      ;;
    --fps)
      (($# >= 2)) || die "--fps requires a value"
      fps="$2"
      shift 2
      ;;
    --rows)
      (($# >= 2)) || die "--rows requires a value"
      rows="$2"
      shift 2
      ;;
    --columns)
      (($# >= 2)) || die "--columns requires a value"
      columns="$2"
      shift 2
      ;;
    --ascii-only)
      ascii_only=1
      shift
      ;;
    --output-dir)
      (($# >= 2)) || die "--output-dir requires a value"
      output_dir="$2"
      shift 2
      ;;
    --external-only)
      internal_metrics_enabled=0
      shift
      ;;

    --no-build)
      build=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

[[ "$terminal_name" =~ ^(yttt|kitty|alacritty|all)$ ]] || \
  die "--terminal must be yttt, kitty, alacritty, or all"
[[ "$trace_mode" =~ ^(none|time|metal|both)$ ]] || \
  die "--trace must be none, time, metal, or both"
[[ "$scenario" =~ ^(full|damage|scroll)$ ]] || \
  die "--scenario must be full, damage, or scroll"
for value_name in duration warmup fps rows columns; do
  value="${!value_name}"
  [[ "$value" =~ ^[0-9]+$ ]] || die "--$value_name must be a non-negative integer"
done
((duration > 0)) || die "--duration must be positive"
((fps > 0)) || die "--fps must be positive"
((rows >= 4)) || die "--rows must be at least 4"
((columns >= 20)) || die "--columns must be at least 20"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workload="$repo_root/scripts/terminal-perf-workload.py"
[[ -x "$workload" ]] || die "workload is not executable: $workload"

if [[ -z "$output_dir" ]]; then
  output_dir="$repo_root/tmp/terminal-perf/$(date -u +%Y%m%dT%H%M%SZ)"
elif [[ "$output_dir" != /* ]]; then
  output_dir="$repo_root/$output_dir"
fi
mkdir -p "$output_dir"

if [[ "$trace_mode" != "none" ]]; then
  [[ "$(uname -s)" == "Darwin" ]] || die "xctrace capture requires macOS"
  xctrace="$(command_path xcrun)"
  [[ -n "$xctrace" ]] || die "xcrun is required for Instruments capture"
fi

if [[ "$terminal_name" == "all" ]]; then
  terminals=(yttt kitty alacritty)
else
  terminals=("$terminal_name")
fi

if [[ " ${terminals[*]} " == *" yttt "* ]]; then
  if ((build)); then
    if ((internal_metrics_enabled)); then
      cargo build -p yttt-terminal --release --features perf-metrics
    else
      cargo build -p yttt-terminal --release
    fi
  fi

  target_root="${CARGO_TARGET_DIR:-$repo_root/target}"
  if [[ "$target_root" != /* ]]; then
    target_root="$repo_root/$target_root"
  fi
  yttt_binary="$target_root/release/yttt-terminal"
  [[ -x "$yttt_binary" ]] || die "yttt binary not found: $yttt_binary"
fi

trace_templates=()
case "$trace_mode" in
  none) trace_templates=(none) ;;
  time) trace_templates=(time) ;;
  metal) trace_templates=(metal) ;;
  both) trace_templates=(time metal) ;;
esac

total_duration=$((warmup + duration))
trace_duration=$((total_duration + 5))



run_one() {
  local terminal="$1"
  local trace_kind="$2"
  local executable workload_shell
  local -a terminal_command workload_command xctrace_command

  local run_dir="$output_dir/$terminal/$trace_kind"
  local workload_metrics="$run_dir/workload.json"
  local internal_metrics="$run_dir/metrics.json"
  local trace_path="$run_dir/$terminal-$trace_kind.trace"
  local start_gate="$run_dir/workload.start"
  local workload_ready="$run_dir/workload.ready"

  local ready_file="$run_dir/yttt.ready"

  local trace_fifo="$run_dir/xctrace-output.fifo"

  mkdir -p "$run_dir"

  workload_command=(
    python3 "$workload"
    --scenario "$scenario"
    --duration "$total_duration"
    --fps "$fps"
    --rows "$rows"
    --columns "$columns"
    --metrics "$workload_metrics"
  )
  [[ ! -e "$start_gate" ]] || die "workload start gate already exists: $start_gate"
  [[ ! -e "$workload_ready" ]] || die "workload ready file already exists: $workload_ready"
  workload_command+=(
    --start-file "$start_gate"
    --ready-file "$workload_ready"
  )

  if [[ "$terminal" == "yttt" ]] && ((internal_metrics_enabled)); then
    [[ ! -e "$ready_file" ]] || die "terminal ready file already exists: $ready_file"
  fi


  if [[ "$trace_kind" != "none" ]]; then
    [[ ! -e "$trace_fifo" ]] || die "xctrace output pipe already exists: $trace_fifo"
  fi



  if ((ascii_only)); then
    workload_command+=(--ascii-only)
  fi

  case "$terminal" in
    yttt)
      executable="$yttt_binary"
      printf -v workload_shell '%q ' "${workload_command[@]}"
      terminal_command=("$executable")
      ;;
    kitty)
      executable="$(command_path kitty)"
      if [[ -z "$executable" && -x /Applications/kitty.app/Contents/MacOS/kitty ]]; then
        executable=/Applications/kitty.app/Contents/MacOS/kitty
      fi
      if [[ -z "$executable" ]]; then
        printf 'skip: kitty is not installed\n' >&2
        return 0
      fi

      terminal_command=(
        "$executable"
        --config NONE
        --title yttt-terminal-perf
        --override confirm_os_window_close=0
        --override remember_window_size=no
        --override initial_window_width=1024
        --override initial_window_height=768
        "${workload_command[@]}"
      )
      ;;
    alacritty)
      executable="$(command_path alacritty)"
      if [[ -z "$executable" && -x /Applications/Alacritty.app/Contents/MacOS/alacritty ]]; then
        executable=/Applications/Alacritty.app/Contents/MacOS/alacritty
      fi
      if [[ -z "$executable" ]]; then
        printf 'skip: alacritty is not installed\n' >&2
        return 0
      fi

      terminal_command=(
        "$executable"
        --title yttt-terminal-perf
        --option window.dimensions.columns=120
        --option window.dimensions.lines=40
        --command "${workload_command[@]}"
      )
      ;;
    *)
      die "unsupported terminal: $terminal"
      ;;
  esac

  printf 'run: terminal=%s trace=%s scenario=%s output=%s\n' \
    "$terminal" "$trace_kind" "$scenario" "$run_dir"

  if [[ "$trace_kind" == "none" ]]; then
    local terminal_pid
    if [[ "$terminal" == "yttt" ]] && ((internal_metrics_enabled)); then
      env \
        SHELL=/bin/bash \
        YTTT_TERMINAL_START_COMMAND="$workload_shell" \
        YTTT_TERMINAL_PERF_OUTPUT="$internal_metrics" \
        YTTT_TERMINAL_PERF_LABEL="$terminal" \
        YTTT_TERMINAL_PERF_SCENARIO="$scenario" \
        YTTT_TERMINAL_PERF_WARMUP_SECONDS="$warmup" \
        YTTT_TERMINAL_PERF_DURATION_SECONDS="$duration" \
        YTTT_TERMINAL_PERF_INPUT_DELAY_MS="$((warmup * 1000 + 500))" \
        YTTT_TERMINAL_PERF_START_FILE="$start_gate" \
        YTTT_TERMINAL_PERF_READY_FILE="$ready_file" \
        YTTT_TERMINAL_PERF_REPORT_INTERVAL_MS=1000 \
        "${terminal_command[@]}" &
      terminal_pid=$!
    elif [[ "$terminal" == "yttt" ]]; then
      env \
        SHELL=/bin/bash \
        YTTT_TERMINAL_BENCHMARK_MODE=1 \
        YTTT_TERMINAL_START_COMMAND="$workload_shell" \
        "${terminal_command[@]}" &
      terminal_pid=$!
    else

      "${terminal_command[@]}" &
      terminal_pid=$!
    fi

    if [[ "$terminal" == "yttt" ]] && ((internal_metrics_enabled)) \
      && ! wait_for_ready_file "$ready_file" "$terminal_pid"; then


      kill -TERM "$terminal_pid" 2>/dev/null || true
      wait "$terminal_pid" 2>/dev/null || true
      die "$terminal did not initialize its performance reporter"
    fi
    if ! wait_for_ready_file "$workload_ready" "$terminal_pid"; then
      kill -TERM "$terminal_pid" 2>/dev/null || true
      wait "$terminal_pid" 2>/dev/null || true
      die "$terminal workload did not reach its start gate"
    fi


    : >"$start_gate"

    local deadline=$((SECONDS + total_duration + 15))
    while kill -0 "$terminal_pid" 2>/dev/null; do
      if [[ "$terminal" != "yttt" && -f "$workload_metrics" ]]; then
        sleep 1
        kill -TERM "$terminal_pid" 2>/dev/null || true
        break
      fi
      if ((SECONDS >= deadline)); then
        kill -TERM "$terminal_pid" 2>/dev/null || true
        break
      fi
      sleep 0.1
    done
    if ! wait "$terminal_pid"; then
      [[ -f "$workload_metrics" ]] || return 1
    fi
    [[ -f "$workload_metrics" ]] || die "$terminal workload did not finish"
    return 0
  fi

  local template
  if [[ "$trace_kind" == "time" ]]; then
    template="Time Profiler"
  else
    template="Metal System Trace"
  fi
  [[ ! -e "$trace_path" ]] || die "trace already exists: $trace_path"
  local target_pid
  if [[ "$terminal" == "yttt" ]] && ((internal_metrics_enabled)); then
    env \
      SHELL=/bin/bash \
      YTTT_TERMINAL_START_COMMAND="$workload_shell" \
      YTTT_TERMINAL_PERF_OUTPUT="$internal_metrics" \
      YTTT_TERMINAL_PERF_LABEL="$terminal" \
      YTTT_TERMINAL_PERF_SCENARIO="$scenario" \
      YTTT_TERMINAL_PERF_WARMUP_SECONDS="$warmup" \
      YTTT_TERMINAL_PERF_DURATION_SECONDS="$duration" \
      YTTT_TERMINAL_PERF_INPUT_DELAY_MS="$((warmup * 1000 + 500))" \
      YTTT_TERMINAL_PERF_START_FILE="$start_gate" \
      YTTT_TERMINAL_PERF_READY_FILE="$ready_file" \
      YTTT_TERMINAL_PERF_REPORT_INTERVAL_MS=1000 \
      YTTT_TERMINAL_KEEP_OPEN_AFTER_EXIT=1 \
      "${terminal_command[@]}" &
    target_pid=$!
  elif [[ "$terminal" == "yttt" ]]; then
    env \
      SHELL=/bin/bash \
      YTTT_TERMINAL_BENCHMARK_MODE=1 \
      YTTT_TERMINAL_START_COMMAND="$workload_shell" \
      YTTT_TERMINAL_KEEP_OPEN_AFTER_EXIT=1 \
      "${terminal_command[@]}" &
    target_pid=$!
  else

    "${terminal_command[@]}" &
    target_pid=$!
  fi

  if [[ "$terminal" == "yttt" ]] && ((internal_metrics_enabled)) \
    && ! wait_for_ready_file "$ready_file" "$target_pid"; then


    kill -TERM "$target_pid" 2>/dev/null || true
    wait "$target_pid" 2>/dev/null || true
    die "$terminal did not initialize its performance reporter"
  fi
  if ! wait_for_ready_file "$workload_ready" "$target_pid"; then
    kill -TERM "$target_pid" 2>/dev/null || true
    wait "$target_pid" 2>/dev/null || true
    die "$terminal workload did not reach its start gate"
  fi



  xctrace_command=(
    xcrun xctrace record
    --template "$template"
    --output "$trace_path"
    --time-limit "${trace_duration}s"
    --no-prompt
    --attach "$target_pid"
  )
  local xctrace_status xctrace_pid
  local workload_started=0
  mkfifo "$trace_fifo"
  set +e
  "${xctrace_command[@]}" >"$trace_fifo" 2>&1 &
  xctrace_pid=$!
  while IFS= read -r trace_line; do
    printf '%s\n' "$trace_line"
    if ((workload_started == 0)) && [[ "$trace_line" == *"Ctrl-C to stop"* ]]; then
      # xctrace briefly suspends the target while attaching. Start the workload only
      # after the target has resumed and the recording pipeline has settled.
      sleep 3

      : >"$start_gate"
      workload_started=1
    fi
  done <"$trace_fifo"
  wait "$xctrace_pid"
  xctrace_status=$?
  set -e
  rm -f "$trace_fifo"


  kill -TERM "$target_pid" 2>/dev/null || true
  for _ in {1..50}; do
    kill -0 "$target_pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL "$target_pid" 2>/dev/null || true
  wait "$target_pid" 2>/dev/null || true
  if ((workload_started == 0)); then
    printf 'error: xctrace never reported an active recording; workload was not started\n' >&2
    return 1
  fi


  if ((xctrace_status != 0)) && [[ ! -d "$trace_path" ]]; then
    return "$xctrace_status"
  fi
  if ((xctrace_status != 0)); then
    printf 'warning: xctrace exited %s after saving %s\n' \
      "$xctrace_status" "$trace_path" >&2
  fi

  xcrun xctrace export \
    --input "$trace_path" \
    --toc \
    --output "$run_dir/toc.xml"
  if [[ "$trace_kind" == "time" ]]; then
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' \
      --output "$run_dir/time-profile.xml"
  else
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="ca-client-present-request"]' \
      --output "$run_dir/present-requests.xml"
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="displayed-surfaces-interval"]' \
      --output "$run_dir/displayed-surfaces.xml"
    xcrun xctrace export \
      --input "$trace_path" \
      --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' \
      --output "$run_dir/time-profile.xml"
  fi
}

for terminal in "${terminals[@]}"; do
  for trace_kind in "${trace_templates[@]}"; do
    run_one "$terminal" "$trace_kind"
  done
done

printf '\nresults: %s\n' "$output_dir"
if [[ -x "$repo_root/scripts/summarize-terminal-perf.py" ]]; then
  "$repo_root/scripts/summarize-terminal-perf.py" "$output_dir"
fi
