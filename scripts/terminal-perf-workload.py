#!/usr/bin/env python3
"""Deterministic ANSI/Unicode terminal workload with optional interactive input echo."""

from __future__ import annotations

import argparse
import codecs
import json
import os
from pathlib import Path
import select
import signal
import sys
import termios
import time
import tty
import unicodedata

CSI = "\x1b["
ALT_SCREEN_ENTER = f"{CSI}?1049h{CSI}?25l{CSI}2J{CSI}H"
ALT_SCREEN_EXIT = f"{CSI}0m{CSI}?25h{CSI}?1049l"
STOP = False


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate repeatable terminal rendering load and echo live keyboard/IME input."
    )
    parser.add_argument(
        "--scenario",
        choices=("full", "damage", "scroll"),
        default="full",
        help="full repaints the viewport, damage updates selected rows, scroll emits new rows",
    )
    parser.add_argument("--duration", type=float, default=20.0)
    parser.add_argument("--startup-delay", type=float, default=0.0)
    parser.add_argument("--start-file", type=Path)
    parser.add_argument("--ready-file", type=Path)



    parser.add_argument("--fps", type=float, default=60.0)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--columns", type=int, default=120)
    parser.add_argument("--metrics", type=Path)
    parser.add_argument(
        "--ascii-only",
        action="store_true",
        help="omit CJK and emoji from rendered content",
    )
    return parser.parse_args()


def cell_width(character: str) -> int:
    if unicodedata.combining(character):
        return 0
    return 2 if unicodedata.east_asian_width(character) in ("W", "F") else 1


def fit_cells(text: str, columns: int) -> str:
    result: list[str] = []
    width = 0
    for character in text:
        character_width = cell_width(character)
        if width + character_width > columns:
            break
        result.append(character)
        width += character_width
    if width < columns:
        result.append(" " * (columns - width))
    return "".join(result)


def styled_line(row: int, frame: int, columns: int, ascii_only: bool) -> str:
    if ascii_only:
        glyphs = "abcdefghijklmnopqrstuvwxyz0123456789"
    else:
        glyphs = "Terminal性能测试中文输入かなカナ한글🙂◆◇▣▢abcdef0123456789"
    shift = (frame * 3 + row * 7) % len(glyphs)
    repeated = (glyphs[shift:] + glyphs[:shift]) * (columns // 2 + 4)
    label = f" {row:02d} frame={frame:06d} "
    body = fit_cells(label + repeated, columns)
    foreground = 31 + (row + frame // 3) % 7
    background = 40 + (row // 3 + frame // 7) % 7
    return f"{CSI}{foreground};{background}m{body}{CSI}0m"


def full_frame(
    frame: int,
    rows: int,
    columns: int,
    ascii_only: bool,
    input_text: str,
) -> bytes:
    lines = [
        f"{CSI}H{CSI}1;38;5;231;48;5;24m"
        + fit_cells(
            f" yttt terminal perf | full | frame {frame:06d} | type English/CJK/IME now ",
            columns,
        )
        + f"{CSI}0m"
    ]
    for row in range(1, max(rows - 2, 1)):
        lines.append(styled_line(row, frame, columns, ascii_only))
    visible_input = input_text[-max(columns - 9, 1) :]
    lines.append(
        f"{CSI}38;5;16;48;5;220m"
        + fit_cells(f" input: {visible_input}", columns)
        + f"{CSI}0m"
    )
    return ("\r\n".join(lines)).encode("utf-8")


def damage_frame(
    frame: int,
    rows: int,
    columns: int,
    ascii_only: bool,
    input_text: str,
) -> bytes:
    if frame == 0:
        return full_frame(frame, rows, columns, ascii_only, input_text)
    targets = (2, max(rows // 3, 2), max((rows * 2) // 3, 3), max(rows - 1, 4))
    fragments = []
    for row in targets:
        fragments.append(f"{CSI}{min(row, rows)};1H")
        fragments.append(styled_line(row, frame, columns, ascii_only))
    visible_input = input_text[-max(columns - 9, 1) :]
    fragments.extend(
        (
            f"{CSI}{rows};1H{CSI}38;5;16;48;5;220m",
            fit_cells(f" input: {visible_input}", columns),
            f"{CSI}0m",
        )
    )
    return "".join(fragments).encode("utf-8")


def scroll_frame(frame: int, columns: int, ascii_only: bool, input_text: str) -> bytes:
    line = styled_line(frame % 97, frame, columns, ascii_only)
    visible_input = input_text[-24:]
    return f"{line} input={visible_input}\r\n".encode("utf-8")


def write_all(file_descriptor: int, payload: bytes) -> int:
    offset = 0
    while offset < len(payload):
        offset += os.write(file_descriptor, payload[offset:])
    return offset


def request_stop(_signum: int, _frame: object) -> None:
    global STOP
    STOP = True


def main() -> int:
    global STOP

    args = parse_args()
    if args.duration <= 0 or args.fps <= 0:
        raise SystemExit("--duration and --fps must be positive")
    if args.rows < 4 or args.columns < 20:
        raise SystemExit("--rows must be >= 4 and --columns must be >= 20")
    if args.startup_delay < 0:
        raise SystemExit("--startup-delay must be non-negative")


    stdin_fd = sys.stdin.fileno()
    stdout_fd = sys.stdout.fileno()
    input_is_tty = os.isatty(stdin_fd)
    previous_termios = termios.tcgetattr(stdin_fd) if input_is_tty else None
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    input_text = ""
    input_bytes = 0
    input_reads = 0
    frames = 0
    bytes_written = 0
    missed_deadlines = 0
    signal.signal(signal.SIGINT, request_stop)
    signal.signal(signal.SIGTERM, request_stop)
    if args.ready_file is not None:
        args.ready_file.parent.mkdir(parents=True, exist_ok=True)
        args.ready_file.touch()

    gate_wait_started = time.monotonic()
    while args.start_file is not None and not args.start_file.is_file() and not STOP:
        time.sleep(0.01)
    gate_wait_seconds = time.monotonic() - gate_wait_started
    if STOP:
        return 130

    if args.startup_delay:
        time.sleep(args.startup_delay)

    started_at_unix_ns = time.time_ns()

    started_at = time.monotonic()
    deadline = started_at + args.duration
    frame_interval = 1.0 / args.fps
    next_frame_at = started_at


    try:
        if input_is_tty:
            tty.setcbreak(stdin_fd)
        bytes_written += write_all(stdout_fd, ALT_SCREEN_ENTER.encode())
        while not STOP:
            now = time.monotonic()
            if now >= deadline:
                break

            if input_is_tty:
                while select.select([stdin_fd], [], [], 0)[0]:
                    incoming = os.read(stdin_fd, 4096)
                    if not incoming:
                        break
                    if b"\x03" in incoming:
                        STOP = True
                        incoming = incoming.replace(b"\x03", b"")
                    input_bytes += len(incoming)
                    input_reads += 1
                    input_text += decoder.decode(incoming)
                    input_text = input_text[-max(args.columns * 4, 256) :]

            now = time.monotonic()
            if now < next_frame_at:
                time.sleep(min(next_frame_at - now, 0.002))
                continue
            if now - next_frame_at >= frame_interval:
                missed_deadlines += int((now - next_frame_at) / frame_interval)
                next_frame_at = now

            if args.scenario == "full":
                payload = full_frame(
                    frames, args.rows, args.columns, args.ascii_only, input_text
                )
            elif args.scenario == "damage":
                payload = damage_frame(
                    frames, args.rows, args.columns, args.ascii_only, input_text
                )
            else:
                payload = scroll_frame(frames, args.columns, args.ascii_only, input_text)
            bytes_written += write_all(stdout_fd, payload)
            frames += 1
            next_frame_at += frame_interval
    finally:
        try:
            write_all(stdout_fd, ALT_SCREEN_EXIT.encode())
        finally:
            if previous_termios is not None:
                termios.tcsetattr(stdin_fd, termios.TCSADRAIN, previous_termios)

    finished_at_unix_ns = time.time_ns()

    elapsed = max(time.monotonic() - started_at, 1e-9)
    result = {
        "schema_version": 1,
        "scenario": args.scenario,
        "ascii_only": args.ascii_only,
        "startup_delay_seconds": args.startup_delay,
        "start_gate_wait_seconds": gate_wait_seconds,
        "started_at_unix_ns": started_at_unix_ns,
        "finished_at_unix_ns": finished_at_unix_ns,
        "requested_fps": args.fps,
        "requested_rows": args.rows,
        "requested_columns": args.columns,
        "elapsed_seconds": elapsed,
        "frames_generated": frames,
        "generator_fps": frames / elapsed,
        "bytes_written": bytes_written,
        "bytes_per_second": bytes_written / elapsed,
        "missed_generator_deadlines": missed_deadlines,
        "input_reads": input_reads,
        "input_bytes": input_bytes,
    }
    if args.metrics:
        args.metrics.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.metrics.with_name(f".{args.metrics.name}.{os.getpid()}.tmp")
        temporary.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        temporary.replace(args.metrics)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
