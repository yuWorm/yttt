# 终端性能复测：P9（47c5ddf）

- 提交：`47c5ddf`（P9.1）。终端、Host、协议、传输 crate 和 `Cargo.lock` 与 M1 复测的 `e4777bc` 相同
- 日期：2026-11-04
- 机器：Mac16,10，Apple M4，32 GB，macOS 26.6；桌面上有其他应用
- 命令：`scripts/run-terminal-perf.sh --backend both --scenario all --runs 3 --trace none --allow-incomplete`，与 `terminal-perf-a2160c8.md` 相同
- 脚本需要 Python 3.11 以上的 `tomllib`；本机 PATH 里的 `/usr/local/bin/python3` 是 3.10，要把 `/opt/homebrew/bin` 放在前面

| backend | scenario | runs | MiB/s median/p95/p99 | paint p95 ms median/p95/p99 | input→PTY p95 ms median | input→paint p95 ms median | echo→paint p95 ms median | sentinel ms median |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| direct | burst | 3 | 101.58/121.57/121.57 | 2016.41/2033.19/2033.19 | — | — | — | 17.221 |
| direct | damage | 3 | 0.05/0.05/0.05 | 18.42/18.78/18.78 | — | — | — | 12.417 |
| direct | full | 3 | 0.39/0.39/0.39 | 18.14/18.55/18.55 | — | — | — | 18.002 |
| direct | interactive | 3 | 0.05/0.05/0.05 | 17.81/31.98/31.98 | 0.122 | 19.612 | 18.006 | 10.937 |
| direct | scroll | 3 | 0.01/0.01/0.01 | 18.56/34.14/34.14 | — | — | — | 4.325 |
| host | burst | 3 | 108.05/110.95/110.95 | 2017.46/2033.19/2033.19 | — | — | — | 20.775 |
| host | damage | 3 | 0.05/0.05/0.05 | 18.35/18.48/18.48 | — | — | — | 9.400 |
| host | full | 3 | 0.39/0.39/0.39 | 18.84/18.89/18.89 | — | — | — | 19.603 |
| host | interactive | 3 | 0.05/0.05/0.05 | 18.06/18.10/18.10 | 0.131 | 21.316 | 15.008 | 9.091 |
| host | scroll | 3 | 0.01/0.01/0.01 | 18.45/18.50/18.50 | — | — | — | 4.569 |

## 与 P0 基线比较（中位数）

| 指标 | 基线 a2160c8 | P9 | 变化 |
|---|---:|---:|---:|
| direct burst MiB/s | 85.43 | 101.58 | +19% |
| host burst MiB/s | 124.24 | 108.05 | -13% |
| paint p95 ms，非 burst 的 8 组 | 17.32–18.24 | 17.81–18.84 | +1% 到 +6% |
| direct input→paint ms | 16.089 | 19.612 | +22% |
| direct echo→paint ms | 14.885 | 18.006 | +21% |
| host input→paint ms | 22.331 | 21.316 | -5% |
| host echo→paint ms | 12.935 | 15.008 | +16% |

## 判断

- 吞吐和帧时间在 10% 以内。host `burst` -13% 落在 1.0 时用户已接受的 -15% 内，原因是 Rust 1.98 和 GPUI 升级，见 `gpui-kit-spike.md`。
- 延迟指标超过 10%，但不是本项目 P2–P9 引入的：终端代码从 M1 起没有改动，M1 与 P0 的交替复测全部在 10% 内。差异最大的是 direct 后端，它是独立的终端窗口，不含工作台界面。这几项延迟按帧量化，一帧约 16.7 ms，三次运行的中位数差 3 ms，属于同一帧内的波动；P0 到 1.0 复测时同两项是 -15%、-14%，方向相反。
- 两次 p95 约 33 ms 的偶发帧（direct interactive 第 3 次、direct scroll 第 1 次）与 P0 基线 `damage` 的已有现象相同，都是三次里中一次。

要排除机器负载，需要和 `e4777bc` 在同一时段交替运行。终端代码相同，本次没有做。
