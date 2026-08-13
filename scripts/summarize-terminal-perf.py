#!/usr/bin/env python3
"""Summarize terminal workload and yttt in-process performance reports."""

from __future__ import annotations

import argparse
import json
import xml.etree.ElementTree as ET
import math
from collections import Counter
from datetime import datetime


from pathlib import Path
from typing import Any


def load_json(path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"warning: cannot read {path}: {error}")
        return None


def nested(data: dict[str, Any] | None, *keys: str) -> Any:
    value: Any = data
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def number(value: Any, digits: int = 2) -> str:
    if value is None:
        return "—"
    if isinstance(value, (int, float)):
        return f"{value:.{digits}f}"
    return str(value)


def integer(value: Any) -> str:
    return "—" if value is None else str(value)


def trace_duration_seconds(toc_path: Path) -> float | None:
    if not toc_path.is_file():
        return None
    try:
        text = ET.parse(toc_path).findtext("./run/info/summary/duration")
        return float(text) if text is not None else None
    except (ET.ParseError, OSError, ValueError):
        return None

def trace_start_unix_ns(toc_path: Path) -> int | None:
    if not toc_path.is_file():
        return None
    try:
        text = ET.parse(toc_path).findtext("./run/info/summary/start-date")
        return int(datetime.fromisoformat(text).timestamp() * 1_000_000_000) if text else None
    except (ET.ParseError, OSError, TypeError, ValueError):
        return None


def workload_trace_window(
    toc_path: Path, workload: dict[str, Any] | None
) -> tuple[int, int] | None:
    trace_start = trace_start_unix_ns(toc_path)
    workload_start = nested(workload, "started_at_unix_ns")
    workload_end = nested(workload, "finished_at_unix_ns")
    if (
        trace_start is None
        or not isinstance(workload_start, int)
        or not isinstance(workload_end, int)
        or workload_end <= workload_start
    ):
        return None
    return (max(0, workload_start - trace_start), workload_end - trace_start)

def trace_target_pid(toc_path: Path) -> int | None:
    if not toc_path.is_file():
        return None
    try:
        process = ET.parse(toc_path).find("./run/info/target/process")
        return int(process.attrib["pid"]) if process is not None else None
    except (ET.ParseError, OSError, KeyError, ValueError):
        return None




def quantile(values: list[float], percentile: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = round((len(ordered) - 1) * percentile)
    return ordered[index]


def parse_time_profile(
    path: Path,
    duration_seconds: float | None,
    timestamp_window: tuple[int, int] | None,
) -> dict[str, Any] | None:

    if not path.is_file():
        return None
    try:
        root = ET.parse(path).getroot()
    except (ET.ParseError, OSError) as error:
        print(f"warning: cannot parse {path}: {error}")
        return None

    identifiers = {
        element.attrib["id"]: element
        for element in root.iter()
        if "id" in element.attrib
    }

    def resolve(element: ET.Element | None) -> ET.Element | None:
        seen: set[str] = set()
        while element is not None and "ref" in element.attrib:
            reference = element.attrib["ref"]
            if reference in seen:
                return None
            seen.add(reference)
            element = identifiers.get(reference)
        return element

    def child(parent: ET.Element | None, tag: str) -> ET.Element | None:
        if parent is None:
            return None
        for candidate in parent:
            resolved = resolve(candidate)
            if resolved is not None and resolved.tag == tag:
                return resolved
        return None

    def frame_info(frame: ET.Element) -> tuple[str, str | None] | None:
        resolved = resolve(frame)
        if resolved is None:
            return None
        binary = child(resolved, "binary")
        return (
            resolved.attrib.get("name", resolved.attrib.get("fmt", "unknown")),
            binary.attrib.get("name") if binary is not None else None,
        )

    total_weight_ns = 0
    sample_count = 0
    application_leaf: Counter[str] = Counter()
    thread_weights: Counter[str] = Counter()
    for row in root.findall(".//row"):
        resolved_children = [resolve(element) for element in row]
        if timestamp_window is not None:
            sample_time = next(
                (
                    element
                    for element in resolved_children
                    if element is not None and element.tag == "sample-time"
                ),
                None,
            )
            try:
                timestamp = (
                    int(sample_time.text)
                    if sample_time is not None and sample_time.text
                    else -1
                )
            except ValueError:
                timestamp = -1
            if not timestamp_window[0] <= timestamp <= timestamp_window[1]:
                continue

        weight = next(
            (
                element
                for element in resolved_children
                if element is not None and element.tag == "weight"
            ),
            None,
        )
        try:
            weight_ns = int(weight.text) if weight is not None and weight.text else 0
        except ValueError:
            weight_ns = 0
        total_weight_ns += weight_ns
        sample_count += 1

        thread = next(
            (
                element
                for element in resolved_children
                if element is not None and element.tag == "thread"
            ),
            None,
        )
        thread_weights[
            thread.attrib.get("fmt", "unknown") if thread is not None else "unknown"
        ] += weight_ns

        tagged_backtrace = next(
            (
                element
                for element in resolved_children
                if element is not None and element.tag == "tagged-backtrace"
            ),
            None,
        )
        backtrace = child(tagged_backtrace, "backtrace")
        if backtrace is None:
            continue
        for frame in backtrace:
            resolved_frame = resolve(frame)
            if resolved_frame is None or resolved_frame.tag != "frame":
                continue
            info = frame_info(resolved_frame)
            if info is not None and info[1] in ("yttt-terminal", "kitty", "alacritty"):
                application_leaf[info[0]] += weight_ns
                break

    cpu_time_ms = total_weight_ns / 1_000_000.0
    average_cpu_percent = (
        cpu_time_ms / (duration_seconds * 10.0)
        if duration_seconds is not None and duration_seconds > 0
        else None
    )

    def weighted_rows(counter: Counter[str]) -> list[dict[str, Any]]:
        return [
            {
                "name": name,
                "cpu_ms": weight_ns / 1_000_000.0,
                "share_percent": (
                    weight_ns * 100.0 / total_weight_ns if total_weight_ns else 0.0
                ),
            }
            for name, weight_ns in counter.most_common(10)
        ]

    return {
        "samples": sample_count,
        "cpu_time_ms": cpu_time_ms,
        "average_cpu_percent": average_cpu_percent,
        "application_leaf_paths": weighted_rows(application_leaf),
        "threads": weighted_rows(thread_weights),
    }


def parse_timestamp_table(
    path: Path,
    target_pid: int | None,
    timestamp_window: tuple[int, int] | None,
) -> dict[str, Any] | None:

    if not path.is_file():
        return None
    try:
        root = ET.parse(path).getroot()
    except (ET.ParseError, OSError) as error:
        print(f"warning: cannot parse {path}: {error}")
        return None
    identifiers = {
        element.attrib["id"]: element
        for element in root.iter()
        if "id" in element.attrib
    }

    def resolve(element: ET.Element) -> ET.Element:
        return identifiers.get(element.attrib.get("ref", ""), element)

    def collect_pids(element: ET.Element, pids: set[int], seen: set[int]) -> None:
        resolved = resolve(element)
        identity = id(resolved)
        if identity in seen:
            return
        seen.add(identity)
        if resolved.tag == "process":
            for child in resolved:
                resolved_child = resolve(child)
                if resolved_child.tag == "pid" and resolved_child.text:
                    try:
                        pids.add(int(resolved_child.text))
                    except ValueError:
                        pass
        for child in resolved:
            collect_pids(child, pids, seen)

    timestamps: list[int] = []
    for row in root.findall(".//row"):
        if not len(row):
            continue
        row_pids: set[int] = set()
        for element in row:
            collect_pids(element, row_pids, set())
        if target_pid is not None and target_pid not in row_pids:
            continue

        timestamp = resolve(row[0])
        try:
            value = int(timestamp.text) if timestamp.text is not None else -1
        except ValueError:
            continue
        if value < 0:
            continue
        if (
            timestamp_window is not None
            and not timestamp_window[0] <= value <= timestamp_window[1]
        ):
            continue
        timestamps.append(value)
    timestamps.sort()
    if not timestamps:
        return None

    intervals_ms = [
        (right - left) / 1_000_000.0
        for left, right in zip(timestamps, timestamps[1:])
        if right > left
    ]
    median = quantile(intervals_ms, 0.50)
    return {
        "events": len(timestamps),
        "interval_p50_ms": median,
        "interval_p95_ms": quantile(intervals_ms, 0.95),
        "interval_p99_ms": quantile(intervals_ms, 0.99),
        "cadence_fps": 1000.0 / median if median else None,
    }



def parse_trace_metrics(
    run_dir: Path, workload: dict[str, Any] | None
) -> dict[str, Any] | None:
    toc_path = run_dir / "toc.xml"
    duration = trace_duration_seconds(toc_path)
    timestamp_window = workload_trace_window(toc_path, workload)
    active_duration = (
        (timestamp_window[1] - timestamp_window[0]) / 1_000_000_000.0
        if timestamp_window is not None
        else None
    )
    target_pid = trace_target_pid(toc_path)
    cpu = parse_time_profile(
        run_dir / "time-profile.xml",
        active_duration if active_duration is not None else duration,
        timestamp_window,
    )
    presents = parse_timestamp_table(
        run_dir / "present-requests.xml", target_pid, timestamp_window
    )
    displayed_surfaces = parse_timestamp_table(
        run_dir / "displayed-surfaces.xml", target_pid, timestamp_window
    )

    if cpu is None and presents is None and displayed_surfaces is None:
        return None
    return {
        "trace_duration_seconds": duration,
        "active_duration_seconds": active_duration,

        "cpu": cpu,
        "present_requests": presents,
        "displayed_surfaces": displayed_surfaces,
    }


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    try:
        documents = [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
            if line
        ]
        return [document for document in documents if isinstance(document, dict)]
    except (OSError, json.JSONDecodeError) as error:
        print(f"warning: cannot read {path}: {error}")
        return []


def finite_numbers(value: Any) -> bool:
    if isinstance(value, float):
        return math.isfinite(value)
    if isinstance(value, dict):
        return all(finite_numbers(item) for item in value.values())
    if isinstance(value, list):
        return all(finite_numbers(item) for item in value)
    return True


def latency_ms(metric: dict[str, Any] | None, percentile: str) -> float | None:
    value = nested(metric, percentile)
    return float(value) if isinstance(value, (int, float)) else None


def nanos_ms(metric: dict[str, Any] | None, percentile: str) -> float | None:
    value = nested(metric, f"{percentile}_nanos")
    return value / 1_000_000.0 if isinstance(value, int) else None


def terminal_host_diagnostics(
    host: dict[str, Any] | None, session_id: str | None
) -> dict[str, Any] | None:
    terminals = nested(host, "terminals")
    if not isinstance(terminals, list):
        return None
    if session_id is not None:
        for terminal in terminals:
            if isinstance(terminal, dict) and terminal.get("session_id") == session_id:
                return terminal
    return terminals[0] if terminals and isinstance(terminals[0], dict) else None

def latest_terminal_host_diagnostics(
    samples: list[dict[str, Any]], session_id: str | None
) -> dict[str, Any] | None:
    for sample in reversed(samples):
        terminal = terminal_host_diagnostics(sample, session_id)
        if terminal is not None:
            return terminal
    return None


def all_queues(host: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not isinstance(host, dict):
        return []
    queues = [queue for queue in host.get("queues", []) if isinstance(queue, dict)]
    for terminal in host.get("terminals", []):
        if isinstance(terminal, dict):
            queues.extend(
                queue for queue in terminal.get("queues", []) if isinstance(queue, dict)
            )
    return queues


def analyze_run(root: Path, run_dir: Path) -> dict[str, Any]:
    relative = run_dir.relative_to(root)
    metadata = load_json(run_dir / "run.json") or {}
    workload = load_json(run_dir / "workload.json")
    report = load_json(run_dir / "metrics.json")
    host_samples = load_jsonl(run_dir / "host-diagnostics.jsonl")
    host = host_samples[-1] if host_samples else None
    parts = relative.parts
    backend = str(metadata.get("backend") or (parts[0] if parts else "unknown"))
    scenario = str(
        metadata.get("scenario")
        or nested(workload, "scenario")
        or (parts[1] if len(parts) > 1 else "unknown")
    )
    session_id = metadata.get("session_id")
    terminal_host = latest_terminal_host_diagnostics(
        host_samples, session_id if isinstance(session_id, str) else None
    )
    metrics = nested(report, "metrics")
    cohort = str(metadata.get("cohort") or "baseline")
    failures: list[str] = []
    warnings: list[str] = []

    if workload is None:
        failures.append("missing workload.json")
    if report is None:
        failures.append("missing metrics.json")
    if (
        not finite_numbers(workload)
        or not finite_numbers(report)
        or not finite_numbers(host_samples)
    ):
        failures.append("metric document contains a non-finite number")
    if report is not None and report.get("phase") != "finished":
        failures.append(f"performance report phase is {report.get('phase')!r}, not 'finished'")

    missed = nested(workload, "missed_generator_deadlines")
    if not isinstance(missed, int):
        failures.append("missing workload missed_generator_deadlines")
    elif missed > 0:
        failures.append(f"workload generator missed {missed} frame deadlines")

    queue_high = nested(metrics, "counters", "read_queue_high_water")
    queue_capacity = nested(metrics, "counters", "read_queue_capacity")
    queue_current = nested(metrics, "counters", "read_queue_current")
    if not isinstance(queue_high, int) or not isinstance(queue_capacity, int):
        failures.append("missing terminal read queue metrics")
    elif queue_high >= queue_capacity:
        failures.append(f"terminal read queue saturated ({queue_high}/{queue_capacity})")
    if queue_current != 0:
        failures.append(f"terminal read queue ended with backlog {queue_current!r}")

    if backend == "host":
        if host is None:
            failures.append("missing host-diagnostics.jsonl")
        queues = all_queues(host)
        if terminal_host is not None and terminal_host not in (nested(host, "terminals") or []):
            queues.extend(
                queue
                for queue in terminal_host.get("queues", [])
                if isinstance(queue, dict)
            )
        if not queues:
            failures.append("Host diagnostics contain no bounded queues")
        for queue in queues:
            name = queue.get("name", "unnamed")
            current = queue.get("current")
            high_water = queue.get("high_water")
            capacity = queue.get("capacity")
            if not all(isinstance(value, int) for value in (current, high_water, capacity)):
                failures.append(f"Host queue {name} is missing depth metrics")
                continue
            if current != 0:
                failures.append(f"Host queue {name} ended with backlog {current}")
            if high_water >= capacity:
                failures.append(f"Host queue {name} saturated ({high_water}/{capacity})")
            dropped = queue.get("dropped")
            resyncs = queue.get("resyncs")
            if not all(isinstance(value, int) for value in (dropped, resyncs)):
                failures.append(f"Host queue {name} is missing drop/resync metrics")
    if backend == "host":
        attachment_queues = [
            queue
            for queue in all_queues(host)
            if queue.get("name") == "attachment_output_bytes"
        ]
        if cohort == "clients-slow":
            if len(attachment_queues) < 2:
                failures.append("slow-client cohort has fewer than two attachment queues")
            resyncing = [
                queue
                for queue in attachment_queues
                if isinstance(queue.get("resyncs"), int) and queue["resyncs"] > 0
            ]
            if len(resyncing) != 1:
                failures.append(
                    f"slow-client cohort resynced {len(resyncing)} attachment queues, expected one"
                )
            if any(
                queue not in resyncing
                and (queue.get("dropped", 0) != 0 or queue.get("resyncs", 0) != 0)
                for queue in attachment_queues
            ):
                failures.append("slow-client cohort affected a live attachment queue")
        elif any(
            queue.get("dropped", 0) != 0 or queue.get("resyncs", 0) != 0
            for queue in attachment_queues
        ):
            failures.append("unexpected attachment drop/resync outside slow-client cohort")

    sentinel_written = nested(workload, "final_sentinel_written_at_unix_ns")
    sentinel_seen = nested(metrics, "counters", "final_sentinel_seen_at_unix_ns")
    sentinel_painted = nested(metrics, "counters", "final_sentinel_painted_at_unix_ns")
    sentinel_ms = None
    if not all(isinstance(value, int) for value in (sentinel_written, sentinel_seen, sentinel_painted)):
        failures.append("missing final sentinel write/merge/paint timestamps")
    elif sentinel_seen < sentinel_written or sentinel_painted < sentinel_seen:
        failures.append("final sentinel timestamps are not monotonic")
    else:
        sentinel_ms = (sentinel_painted - sentinel_written) / 1_000_000.0
        if sentinel_ms > 1_000.0:
            failures.append(f"final sentinel reached paint in {sentinel_ms:.2f} ms (>1000 ms)")

    scenario_latency = (
        scenario in {"full", "damage", "scroll", "interactive"}
        and cohort != "lifecycle-exited"
    )
    frame = nested(metrics, "latencies", "paint_frame_interval_ms")
    frame_p50 = latency_ms(frame, "p50_ms")
    frame_p95 = latency_ms(frame, "p95_ms")
    if scenario_latency:
        if frame_p50 is None or frame_p95 is None:
            failures.append("missing paint frame interval percentiles")
        elif frame_p50 > 18.5 or frame_p95 > 25.0:
            failures.append(
                f"paint cadence is below approximately 60 FPS "
                f"(p50={frame_p50:.2f} ms, p95={frame_p95:.2f} ms)"
            )

    input_to_pty = (
        nanos_ms(nested(terminal_host, "input_to_pty"), "p95")
        if backend == "host"
        else latency_ms(nested(metrics, "latencies", "input_to_pty_write_ms"), "p95_ms")
    )
    echo_paint = latency_ms(
        nested(metrics, "latencies", "echo_to_first_paint_ms"), "p95_ms"
    )
    if scenario == "interactive":
        input_samples = (
            nested(terminal_host, "input_to_pty", "samples")
            if backend == "host"
            else nested(metrics, "latencies", "input_to_pty_write_ms", "samples")
        )
        echo_samples = nested(
            metrics, "latencies", "echo_to_first_paint_ms", "samples"
        )
        if not isinstance(input_samples, int) or input_samples < 600:
            failures.append(f"interactive input-to-PTY sample count is {input_samples!r} (<600)")
        if not isinstance(echo_samples, int) or echo_samples < 600:
            failures.append(f"interactive echo-to-paint sample count is {echo_samples!r} (<600)")
        if input_to_pty is None:
            failures.append("missing input-to-PTY p95")
        elif input_to_pty > 0.5:
            failures.append(f"input-to-PTY p95 is {input_to_pty:.3f} ms (>0.5 ms)")

    traces = sorted(str(path.relative_to(root)) for path in run_dir.glob("*.trace"))
    trace_metrics = parse_trace_metrics(run_dir, workload)
    return {
        "backend": backend,
        "scenario": scenario,
        "run": metadata.get("run"),
        "cohort": cohort,
        "run_directory": str(relative),
        "metadata": metadata,
        "workload": workload,
        "performance_report": report,
        "host_diagnostics": host,
        "trace_metrics": trace_metrics,
        "traces": traces,
        "derived": {
            "mib_per_second": (
                nested(workload, "bytes_per_second") / (1024 * 1024)
                if isinstance(nested(workload, "bytes_per_second"), (int, float))
                else None
            ),
            "paint_interval_p50_ms": frame_p50,
            "paint_interval_p95_ms": frame_p95,
            "input_to_pty_p95_ms": input_to_pty,
            "echo_to_paint_p95_ms": echo_paint,
            "final_sentinel_to_paint_ms": sentinel_ms,
        },
        "warnings": warnings,
        "failures": failures,
    }


def aggregate_values(values: list[float]) -> dict[str, float | int | None]:
    return {
        "samples": len(values),
        "median": quantile(values, 0.50),
        "p95": quantile(values, 0.95),
        "p99": quantile(values, 0.99),
    }


def aggregate_runs(runs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for run in runs:
        if run.get("cohort") != "baseline":
            continue
        grouped.setdefault((run["backend"], run["scenario"]), []).append(run)
    aggregates = []
    for (backend, scenario), group in sorted(grouped.items()):
        metric_names = (
            "mib_per_second",
            "paint_interval_p50_ms",
            "paint_interval_p95_ms",
            "input_to_pty_p95_ms",
            "echo_to_paint_p95_ms",
            "final_sentinel_to_paint_ms",
        )
        aggregates.append(
            {
                "backend": backend,
                "scenario": scenario,
                "runs": len(group),
                "metrics": {
                    name: aggregate_values(
                        [
                            float(run["derived"][name])
                            for run in group
                            if isinstance(run["derived"][name], (int, float))
                        ]
                    )
                    for name in metric_names
                },
            }
        )
    return aggregates


def enforce_matrix_coverage(
    root: Path, runs: list[dict[str, Any]], failures: list[str]
) -> None:
    expected = {
        "panes-4": {"panes": 4, "clients": "one", "desktop_lifecycle": "open"},
        "panes-16": {"panes": 16, "clients": "one", "desktop_lifecycle": "open"},
        "clients-two": {"panes": 1, "clients": "two", "desktop_lifecycle": "open"},
        "clients-slow": {"panes": 1, "clients": "slow", "desktop_lifecycle": "open"},
        "lifecycle-exited": {
            "panes": 1,
            "clients": "one",
            "desktop_lifecycle": "exited",
        },
        "lifecycle-reopened": {
            "panes": 1,
            "clients": "one",
            "desktop_lifecycle": "reopened",
        },
    }
    for cohort, contract in expected.items():
        cohort_runs = [
            run
            for run in runs
            if run.get("cohort") == cohort
            and run.get("backend") == "host"
            and run.get("scenario") == "damage"
        ]
        if len(cohort_runs) < 5:
            failures.append(
                f"{cohort} coverage requires at least 5 Host runs (got {len(cohort_runs)})"
            )
            continue
        for run in cohort_runs:
            metadata = run.get("metadata")
            for field, expected_value in contract.items():
                if nested(metadata, field) != expected_value:
                    failures.append(
                        f"{run['run_directory']}: {field} is "
                        f"{nested(metadata, field)!r}, expected {expected_value!r}"
                    )

    host_only = []
    for path in root.glob("resources/host-only/run-*/run.json"):
        document = load_json(path)
        if document is not None:
            host_only.append(document)
    if len(host_only) < 5:
        failures.append(
            f"zero-pane Host coverage requires at least 5 runs (got {len(host_only)})"
        )
    for document in host_only:
        if document.get("panes") != 0 or document.get("clients") != "none":
            failures.append("Host-only resource probe metadata is not zero-pane/headless")
        if not isinstance(document.get("samples"), int) or document["samples"] < 5:
            failures.append("Host-only resource probe contains fewer than 5 idle samples")


def enforce_cross_backend_thresholds(
    aggregates: list[dict[str, Any]], failures: list[str]
) -> None:
    by_key = {
        (aggregate["backend"], aggregate["scenario"]): aggregate
        for aggregate in aggregates
    }
    for scenario in ("full", "damage", "scroll", "burst", "interactive"):
        direct = by_key.get(("direct", scenario))
        host = by_key.get(("host", scenario))
        if direct is None or host is None:
            failures.append(f"missing direct/host aggregate for {scenario}")
            continue
        if direct["runs"] < 5 or host["runs"] < 5:
            failures.append(
                f"{scenario} requires at least 5 direct and 5 host runs "
                f"(got {direct['runs']}/{host['runs']})"
            )
        direct_throughput = nested(direct, "metrics", "mib_per_second", "median")
        host_throughput = nested(host, "metrics", "mib_per_second", "median")
        if not isinstance(direct_throughput, (int, float)) or not isinstance(
            host_throughput, (int, float)
        ):
            failures.append(f"missing throughput medians for {scenario}")
        elif direct_throughput > 0 and host_throughput < direct_throughput * 0.90:
            regression = (1.0 - host_throughput / direct_throughput) * 100.0
            failures.append(f"{scenario} Host throughput regressed {regression:.2f}% (>10%)")

    direct_echo = nested(
        by_key.get(("direct", "interactive")),
        "metrics",
        "echo_to_paint_p95_ms",
        "median",
    )
    host_echo = nested(
        by_key.get(("host", "interactive")),
        "metrics",
        "echo_to_paint_p95_ms",
        "median",
    )
    if not isinstance(direct_echo, (int, float)) or not isinstance(
        host_echo, (int, float)
    ):
        failures.append("missing interactive echo-to-paint baseline medians")
    elif host_echo > direct_echo + 3.0:
        failures.append(
            f"Host echo-to-paint p95 median adds {host_echo - direct_echo:.2f} ms (>3 ms)"
        )


def enforce_resource_thresholds(root: Path, failures: list[str]) -> dict[str, Any] | None:
    resources = load_json(root / "resources.json")
    if resources is None:
        failures.append("missing resources.json")
        return None
    host_cpu = nested(resources, "host_only", "idle_cpu_percent")
    host_rss = nested(resources, "host_only", "rss_bytes")
    memory_metric = resources.get("acceptance_memory_metric", "rss_bytes")
    if memory_metric not in {"rss_bytes", "physical_footprint_bytes"}:
        failures.append(f"unknown acceptance memory metric {memory_metric!r}")
        memory_metric = "rss_bytes"
    direct_memory = nested(resources, "direct_one_pane", memory_metric)
    combined_memory = nested(resources, "combined_one_pane", memory_metric)
    if not isinstance(host_cpu, (int, float)) or not math.isfinite(host_cpu):
        failures.append("missing finite Host-only idle CPU")
    elif host_cpu >= 0.5:
        failures.append(f"Host-only idle CPU is {host_cpu:.3f}% (>=0.5%)")
    if not isinstance(host_rss, int):
        failures.append("missing Host-only RSS")
    if not isinstance(direct_memory, int) or not isinstance(combined_memory, int):
        failures.append(f"missing direct/combined one-pane {memory_metric}")
    elif combined_memory - direct_memory > 15 * 1024 * 1024:
        delta = (combined_memory - direct_memory) / (1024 * 1024)
        label = (
            "physical footprint"
            if memory_metric == "physical_footprint_bytes"
            else "RSS"
        )
        failures.append(
            f"combined one-pane {label} increment is {delta:.2f} MiB (>15 MiB)"
        )
    return resources


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_root", type=Path)
    parser.add_argument(
        "--output",
        type=Path,
        help="consolidated JSON path (default: <result_root>/summary.json)",
    )
    parser.add_argument(
        "--allow-incomplete",
        action="store_true",
        help="summarize partial smoke data without enforcing the full acceptance matrix",
    )
    args = parser.parse_args()
    root = args.result_root.resolve()
    if not root.is_dir():
        raise SystemExit(f"result root does not exist: {root}")

    run_dirs = sorted({path.parent for path in root.rglob("workload.json")})
    if not run_dirs:
        raise SystemExit(f"no terminal performance runs found under {root}")
    runs = [analyze_run(root, run_dir) for run_dir in run_dirs]
    aggregates = aggregate_runs(runs)
    failures = [
        f"{run['run_directory']}: {failure}"
        for run in runs
        for failure in run["failures"]
    ]
    resources = None
    if not args.allow_incomplete:
        enforce_cross_backend_thresholds(aggregates, failures)
        resources = enforce_resource_thresholds(root, failures)
        enforce_matrix_coverage(root, runs, failures)
        traced = {
            (run["backend"], run["scenario"])
            for run in runs
            if run.get("cohort") == "baseline" and run["traces"]
        }
        for backend in ("direct", "host"):
            for scenario in ("full", "damage", "scroll", "burst", "interactive"):
                if (backend, scenario) not in traced:
                    failures.append(
                        f"missing raw trace capture for {backend}/{scenario}"
                    )

    print(
        "| backend | scenario | runs | MiB/s median/p95/p99 | "
        "paint p95 ms median/p95/p99 | input→PTY p95 ms median | "
        "echo→paint p95 ms median | sentinel ms median |"
    )
    print("|---|---|---:|---:|---:|---:|---:|---:|")
    for aggregate in aggregates:
        metrics = aggregate["metrics"]

        def triple(name: str) -> str:
            metric = metrics[name]
            return "/".join(number(metric[key]) for key in ("median", "p95", "p99"))

        print(
            f"| {aggregate['backend']} | {aggregate['scenario']} | "
            f"{aggregate['runs']} | {triple('mib_per_second')} | "
            f"{triple('paint_interval_p95_ms')} | "
            f"{number(nested(metrics, 'input_to_pty_p95_ms', 'median'), 3)} | "
            f"{number(nested(metrics, 'echo_to_paint_p95_ms', 'median'), 3)} | "
            f"{number(nested(metrics, 'final_sentinel_to_paint_ms', 'median'), 3)} |"
        )

    summary = {
        "schema_version": 2,
        "result_root": str(root),
        "valid": not failures,
        "runs": runs,
        "aggregates": aggregates,
        "resources": resources,
        "failures": failures,
    }
    output = args.output.resolve() if args.output else root / "summary.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.tmp")
    temporary.write_text(
        json.dumps(summary, indent=2, allow_nan=False) + "\n", encoding="utf-8"
    )
    temporary.replace(output)
    print(f"\nsummary JSON: {output}")
    if failures:
        print("\nacceptance failures:")
        for failure in failures:
            print(f"- {failure}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
