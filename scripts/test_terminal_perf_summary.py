#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("summarize-terminal-perf.py")
SPEC = importlib.util.spec_from_file_location("summarize_terminal_perf", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
summary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(summary)


class TerminalPerformanceSummaryTests(unittest.TestCase):
    def test_run_analysis_rejects_backlog_saturation_and_late_sentinel(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            run_dir = root / "host" / "damage" / "run-01"
            run_dir.mkdir(parents=True)
            self.write_json(
                run_dir / "run.json",
                {
                    "backend": "host",
                    "scenario": "damage",
                    "run": 1,
                    "session_id": "terminal-1",
                },
            )
            self.write_json(
                run_dir / "workload.json",
                {
                    "scenario": "damage",
                    "bytes_per_second": 1_000_000,
                    "missed_generator_deadlines": 1,
                    "final_sentinel_written_at_unix_ns": 1_000_000_000,
                },
            )
            self.write_json(
                run_dir / "metrics.json",
                {
                    "phase": "finished",
                    "metrics": {
                        "counters": {
                            "read_queue_high_water": 8,
                            "read_queue_capacity": 8,
                            "read_queue_current": 1,
                            "final_sentinel_seen_at_unix_ns": 1_100_000_000,
                            "final_sentinel_painted_at_unix_ns": 2_100_000_000,
                        },
                        "latencies": {
                            "paint_frame_interval_ms": {
                                "p50_ms": 20.0,
                                "p95_ms": 30.0,
                            }
                        },
                    },
                },
            )
            host_snapshot = {
                "queues": [{"name": "service", "current": 1, "high_water": 4, "capacity": 4}],
                "terminals": [
                    {
                        "session_id": "terminal-1",
                        "queues": [
                            {
                                "name": "terminal-writer",
                                "current": 2,
                                "high_water": 8,
                                "capacity": 8,
                            }
                        ],
                    }
                ],
            }
            (run_dir / "host-diagnostics.jsonl").write_text(
                json.dumps(host_snapshot) + "\n", encoding="utf-8"
            )

            analyzed = summary.analyze_run(root, run_dir)
            failures = "\n".join(analyzed["failures"])
            self.assertIn("missed 1 frame deadlines", failures)
            self.assertIn("terminal read queue saturated", failures)
            self.assertIn("terminal read queue ended with backlog", failures)
            self.assertIn("Host queue service ended with backlog", failures)
            self.assertIn("Host queue terminal-writer saturated", failures)
            self.assertIn("final sentinel reached paint", failures)
            self.assertIn("paint cadence is below", failures)

            self.write_json(
                run_dir / "run.json",
                {
                    "backend": "host",
                    "scenario": "damage",
                    "run": 1,
                    "cohort": "lifecycle-exited",
                    "session_id": "terminal-1",
                },
            )
            exited_failures = "\n".join(
                summary.analyze_run(root, run_dir)["failures"]
            )
            self.assertNotIn("paint cadence is below", exited_failures)

    def test_interactive_analysis_rejects_non_finite_and_low_sample_metrics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            run_dir = root / "direct" / "interactive" / "run-01"
            run_dir.mkdir(parents=True)
            self.write_json(
                run_dir / "run.json",
                {"backend": "direct", "scenario": "interactive", "run": 1},
            )
            (run_dir / "workload.json").write_text(
                json.dumps(
                    {
                        "scenario": "interactive",
                        "bytes_per_second": float("nan"),
                        "missed_generator_deadlines": 0,
                        "final_sentinel_written_at_unix_ns": 1_000_000_000,
                    },
                    allow_nan=True,
                )
                + "\n",
                encoding="utf-8",
            )
            self.write_json(
                run_dir / "metrics.json",
                {
                    "phase": "finished",
                    "metrics": {
                        "counters": {
                            "read_queue_high_water": 1,
                            "read_queue_capacity": 8,
                            "read_queue_current": 0,
                            "final_sentinel_seen_at_unix_ns": 1_010_000_000,
                            "final_sentinel_painted_at_unix_ns": 1_020_000_000,
                        },
                        "latencies": {
                            "paint_frame_interval_ms": {
                                "p50_ms": 16.5,
                                "p95_ms": 20.0,
                            },
                            "input_to_pty_write_ms": {
                                "samples": 599,
                                "p95_ms": 0.1,
                            },
                            "echo_to_first_paint_ms": {
                                "samples": 599,
                                "p95_ms": 2.0,
                            },
                            "input_to_first_paint_ms": {
                                "samples": 599,
                                "p95_ms": 12.0,
                            },
                        },
                    },
                },
            )

            analyzed = summary.analyze_run(root, run_dir)
            failures = "\n".join(analyzed["failures"])
            self.assertIn("metric document contains a non-finite number", failures)
            self.assertIn("interactive input-to-PTY sample count is 599", failures)
            self.assertIn("interactive echo-to-paint sample count is 599", failures)
            self.assertIn("interactive input-to-paint sample count is 599", failures)

    def test_cross_backend_thresholds_reject_missing_runs_and_regressions(self) -> None:
        def aggregate(
            backend: str,
            scenario: str,
            runs: int,
            throughput: float,
            echo: float | None = None,
            input_paint: float | None = None,
        ) -> dict[str, object]:
            return {
                "backend": backend,
                "scenario": scenario,
                "runs": runs,
                "metrics": {
                    "mib_per_second": {"median": throughput},
                    "echo_to_paint_p95_ms": {"median": echo},
                    "input_to_paint_p95_ms": {"median": input_paint},
                },
            }

        aggregates: list[dict[str, object]] = []
        for scenario in ("full", "damage", "scroll", "burst", "interactive"):
            aggregates.append(aggregate("direct", scenario, 4, 100.0, 10.0, 10.0))
            aggregates.append(aggregate("host", scenario, 4, 80.0, 14.0, 21.0))
        failures: list[str] = []
        summary.enforce_cross_backend_thresholds(aggregates, failures)
        summary.enforce_interactive_latency_threshold(aggregates, failures)
        joined = "\n".join(failures)
        self.assertIn("requires at least 5 direct and 5 host runs", joined)
        self.assertIn("Host throughput regressed 20.00%", joined)
        self.assertIn("Host echo-to-paint p95 median adds 4.00 ms", joined)
        self.assertIn("Host input-to-paint p95 median is 2.10x Direct", joined)

    def test_resource_thresholds_reject_busy_or_oversized_host(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            self.write_json(
                root / "resources.json",
                {
                    "host_only": {
                        "idle_cpu_percent": 0.5,
                        "rss_bytes": 20 * 1024 * 1024,
                    },
                    "direct_one_pane": {"rss_bytes": 100 * 1024 * 1024},
                    "combined_one_pane": {"rss_bytes": 116 * 1024 * 1024},
                },
            )
            failures: list[str] = []
            summary.enforce_resource_thresholds(root, failures)
            joined = "\n".join(failures)
            self.assertIn("Host-only idle CPU is 0.500%", joined)
            self.assertIn("combined one-pane RSS increment is 16.00 MiB", joined)

    def test_resource_thresholds_use_physical_footprint_when_available(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            self.write_json(
                root / "resources.json",
                {
                    "acceptance_memory_metric": "physical_footprint_bytes",
                    "host_only": {
                        "idle_cpu_percent": 0.1,
                        "rss_bytes": 20 * 1024 * 1024,
                        "physical_footprint_bytes": 10 * 1024 * 1024,
                    },
                    "direct_one_pane": {
                        "rss_bytes": 80 * 1024 * 1024,
                        "physical_footprint_bytes": 40 * 1024 * 1024,
                    },
                    "combined_one_pane": {
                        "rss_bytes": 104 * 1024 * 1024,
                        "physical_footprint_bytes": 50 * 1024 * 1024,
                    },
                },
            )
            failures: list[str] = []
            summary.enforce_resource_thresholds(root, failures)
            self.assertEqual(failures, [])

    def test_matrix_coverage_requires_five_runs_for_every_host_cohort(self) -> None:
        contracts = {
            "panes-4": (4, "one", "open"),
            "panes-16": (16, "one", "open"),
            "clients-two": (1, "two", "open"),
            "clients-slow": (1, "slow", "open"),
            "lifecycle-exited": (1, "one", "exited"),
            "lifecycle-reopened": (1, "one", "reopened"),
        }
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            runs: list[dict[str, object]] = []
            for cohort, (panes, clients, lifecycle) in contracts.items():
                for run_number in range(1, 6):
                    runs.append(
                        {
                            "cohort": cohort,
                            "backend": "host",
                            "scenario": "damage",
                            "run_directory": f"{cohort}/host/damage/run-{run_number:02d}",
                            "metadata": {
                                "panes": panes,
                                "clients": clients,
                                "desktop_lifecycle": lifecycle,
                            },
                        }
                    )
            for run_number in range(1, 6):
                run_dir = root / "resources" / "host-only" / f"run-{run_number:02d}"
                run_dir.mkdir(parents=True)
                self.write_json(
                    run_dir / "run.json",
                    {
                        "panes": 0,
                        "clients": "none",
                        "samples": 6,
                    },
                )

            failures: list[str] = []
            summary.enforce_matrix_coverage(root, runs, failures)
            self.assertEqual(failures, [])

            failures = []
            summary.enforce_matrix_coverage(root, runs[:-1], failures)
            self.assertIn(
                "lifecycle-reopened coverage requires at least 5 Host runs (got 4)",
                failures,
            )

    @staticmethod
    def write_json(path: Path, document: dict[str, object]) -> None:
        path.write_text(json.dumps(document, allow_nan=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
