#!/usr/bin/env python3
"""Summarize terminal workload and yttt in-process performance reports."""

from __future__ import annotations

import argparse
import json
import xml.etree.ElementTree as ET
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


def analyze_run(
    root: Path,
    run_dir: Path,
    workload: dict[str, Any] | None,
    report: dict[str, Any] | None,
) -> dict[str, Any]:
    relative = run_dir.relative_to(root)
    terminal = relative.parts[0] if relative.parts else run_dir.name
    capture = relative.parts[1] if len(relative.parts) > 1 else "run"
    metrics = nested(report, "metrics")
    warnings: list[str] = []

    missed = nested(workload, "missed_generator_deadlines")
    if isinstance(missed, int) and missed > 0:
        warnings.append(f"workload generator missed {missed} frame deadlines")

    generated_frames = nested(workload, "frames_generated")
    painted_frames = nested(metrics, "counters", "painted_frames")
    if (
        isinstance(generated_frames, int)
        and generated_frames > 0
        and painted_frames == 0
    ):
        warnings.append("no terminal paint occurred during an active workload")

    queue_high = nested(metrics, "counters", "read_queue_high_water")
    queue_capacity = nested(metrics, "counters", "read_queue_capacity")
    if (
        isinstance(queue_high, int)
        and isinstance(queue_capacity, int)
        and queue_high >= queue_capacity
    ):
        warnings.append(f"PTY read queue saturated ({queue_high}/{queue_capacity})")

    frame_p95 = nested(metrics, "latencies", "paint_frame_interval_ms", "p95_ms")
    if isinstance(frame_p95, (int, float)) and frame_p95 > 16.67:
        warnings.append(f"paint interval p95 is {frame_p95:.2f} ms (>16.67 ms)")

    parser_wait_p99 = nested(metrics, "latencies", "parser_lock_wait_ms", "p99_ms")
    if isinstance(parser_wait_p99, (int, float)) and parser_wait_p99 > 2.0:
        warnings.append(f"Term lock wait p99 is {parser_wait_p99:.2f} ms (>2 ms)")

    prepaint_p95 = nested(metrics, "latencies", "prepaint_ms", "p95_ms")
    if isinstance(prepaint_p95, (int, float)) and prepaint_p95 > 8.0:
        warnings.append(f"prepaint p95 is {prepaint_p95:.2f} ms (>8 ms)")

    dropped = nested(metrics, "counters", "dropped_correlations")
    if isinstance(dropped, int) and dropped > 0:
        warnings.append(f"dropped {dropped} timing correlations")

    traces = sorted(str(path.relative_to(root)) for path in run_dir.glob("*.trace"))
    trace_metrics = parse_trace_metrics(run_dir, workload)


    return {
        "terminal": terminal,
        "capture": capture,
        "run_directory": str(relative),
        "workload": workload,
        "performance_report": report,
        "trace_metrics": trace_metrics,

        "traces": traces,
        "warnings": warnings,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_root", type=Path)
    parser.add_argument(
        "--output",
        type=Path,
        help="consolidated JSON path (default: <result_root>/summary.json)",
    )
    args = parser.parse_args()
    root = args.result_root.resolve()
    if not root.is_dir():
        raise SystemExit(f"result root does not exist: {root}")

    run_dirs = sorted(
        {
            path.parent
            for pattern in ("workload.json", "metrics.json", "*.trace")
            for path in root.glob(f"*/*/{pattern}")
        }
    )
    runs = [
        analyze_run(
            root,
            run_dir,
            load_json(run_dir / "workload.json"),
            load_json(run_dir / "metrics.json"),
        )
        for run_dir in run_dirs
    ]
    if not runs:
        raise SystemExit(f"no terminal performance runs found under {root}")

    print(
        "| terminal | capture | generator FPS | MiB/s | paint FPS | "
        "paint interval p50/p95/p99 ms | parser p95/p99 ms | "
        "prepaint p95/p99 ms | paint p95/p99 ms | queue HWM | warnings |"
    )
    print(
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|"
    )
    for run in runs:
        workload = run["workload"]
        report = run["performance_report"]
        metrics = nested(report, "metrics")
        bytes_per_second = nested(workload, "bytes_per_second")
        mib_per_second = (
            bytes_per_second / (1024 * 1024)
            if isinstance(bytes_per_second, (int, float))
            else None
        )
        frame = nested(metrics, "latencies", "paint_frame_interval_ms")
        parser_metric = nested(metrics, "latencies", "parser_batch_ms")
        prepaint = nested(metrics, "latencies", "prepaint_ms")
        paint = nested(metrics, "latencies", "paint_ms")
        queue = nested(metrics, "counters", "read_queue_high_water")
        queue_capacity = nested(metrics, "counters", "read_queue_capacity")
        queue_text = (
            f"{queue}/{queue_capacity}"
            if queue is not None and queue_capacity is not None
            else "—"
        )
        warning_text = "; ".join(run["warnings"]) or "none"
        print(
            f"| {run['terminal']} | {run['capture']} | "
            f"{number(nested(workload, 'generator_fps'))} | {number(mib_per_second)} | "
            f"{number(nested(metrics, 'paint_fps'))} | "
            f"{number(nested(frame, 'p50_ms'))}/{number(nested(frame, 'p95_ms'))}/"
            f"{number(nested(frame, 'p99_ms'))} | "
            f"{number(nested(parser_metric, 'p95_ms'))}/"
            f"{number(nested(parser_metric, 'p99_ms'))} | "
            f"{number(nested(prepaint, 'p95_ms'))}/"
            f"{number(nested(prepaint, 'p99_ms'))} | "
            f"{number(nested(paint, 'p95_ms'))}/"
            f"{number(nested(paint, 'p99_ms'))} | {queue_text} | {warning_text} |"
        )

    print("\nInput latency (yttt runs with sampled input):")
    print(
        "| capture | events | input→PTY p50/p95/p99 ms | "
        "input→echo parse p50/p95/p99 ms | input→echo paint p50/p95/p99 ms | "
        "IME preedit→paint p50/p95/p99 ms |"
    )
    print("|---|---:|---:|---:|---:|---:|")
    for run in runs:
        report = run["performance_report"]
        metrics = nested(report, "metrics")
        if metrics is None:
            continue

        def triple(name: str) -> str:
            metric = nested(metrics, "latencies", name)
            return "/".join(
                number(nested(metric, key)) for key in ("p50_ms", "p95_ms", "p99_ms")
            )

        print(
            f"| {run['capture']} | {integer(nested(metrics, 'counters', 'input_events'))} | "
            f"{triple('input_to_pty_write_ms')} | "
            f"{triple('input_to_echo_parse_ms')} | "
            f"{triple('input_to_first_paint_after_echo_ms')} | "
            f"{triple('ime_preedit_to_paint_ms')} |"
        )

    traced_runs = [run for run in runs if run["trace_metrics"] is not None]
    if traced_runs:
        print("\nInstruments trace metrics:")
        print(
            "| terminal | capture | trace seconds | active seconds | CPU time ms | "
            "active CPU % | present requests | present cadence FPS | "
            "present interval p50/p95/p99 ms | surface display intervals | "
            "surface cadence FPS | surface interval p50/p95/p99 ms |"
        )
        print(
            "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
        )


        for run in traced_runs:
            trace = run["trace_metrics"]
            cpu = nested(trace, "cpu")
            presents = nested(trace, "present_requests")
            displayed = nested(trace, "displayed_surfaces")

            print(
                f"| {run['terminal']} | {run['capture']} | "
                f"{number(nested(trace, 'trace_duration_seconds'))} | "
                f"{number(nested(trace, 'active_duration_seconds'))} | "
                f"{number(nested(cpu, 'cpu_time_ms'))} | "
                f"{number(nested(cpu, 'average_cpu_percent'))} | "
                f"{integer(nested(presents, 'events'))} | "
                f"{number(nested(presents, 'cadence_fps'))} | "
                f"{number(nested(presents, 'interval_p50_ms'))}/"
                f"{number(nested(presents, 'interval_p95_ms'))}/"
                f"{number(nested(presents, 'interval_p99_ms'))} | "
                f"{integer(nested(displayed, 'events'))} | "
                f"{number(nested(displayed, 'cadence_fps'))} | "
                f"{number(nested(displayed, 'interval_p50_ms'))}/"
                f"{number(nested(displayed, 'interval_p95_ms'))}/"
                f"{number(nested(displayed, 'interval_p99_ms'))} |"
            )
            paths = nested(cpu, "application_leaf_paths")
            if isinstance(paths, list) and paths:
                print(f"\nTop application-side CPU paths for {run['terminal']}/{run['capture']}:")
                for path in paths[:5]:
                    print(
                        f"- {number(path.get('cpu_ms'))} ms "
                        f"({number(path.get('share_percent'))}%): {path.get('name')}"
                    )

    summary = {"schema_version": 1, "result_root": str(root), "runs": runs}
    output = args.output.resolve() if args.output else root / "summary.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.tmp")
    temporary.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    temporary.replace(output)
    print(f"\nsummary JSON: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
