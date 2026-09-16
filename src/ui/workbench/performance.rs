use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{App, AppContext, Context, Entity, Global, Subscription, Task, Window};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::WorkbenchView;
use crate::{
    config::bars::{ShellBarModule, ShellBarsSettings},
    ui::i18n::UiTextKey,
};

const PERFORMANCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const MEBIBYTE_BYTES: f64 = 1024.0 * 1024.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerformanceMetricInfo {
    pub value: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationPerformanceInfo {
    pub projects: PerformanceMetricInfo,
    pub terminals: PerformanceMetricInfo,
    pub tabs: PerformanceMetricInfo,
    pub editors: PerformanceMetricInfo,
    pub cpu: PerformanceMetricInfo,
    pub memory: PerformanceMetricInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemPerformanceInfo {
    pub cpu: PerformanceMetricInfo,
    pub memory: PerformanceMetricInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerformanceInfo {
    pub application: Option<ApplicationPerformanceInfo>,
    pub system: Option<SystemPerformanceInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ApplicationPerformanceSample {
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SystemPerformanceSample {
    cpu_percent: f32,
    memory_percent: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PerformanceSample {
    application: Option<ApplicationPerformanceSample>,
    system: Option<SystemPerformanceSample>,
}

pub(super) struct PerformanceMonitor {
    sample: Rc<Cell<Option<PerformanceSample>>>,
    _task: Option<Task<()>>,
}

struct SharedPerformanceMonitor(Entity<PerformanceMonitor>);

impl Global for SharedPerformanceMonitor {}

impl PerformanceMonitor {
    pub(super) fn shared(cx: &mut App) -> Entity<Self> {
        if let Some(shared) = cx.try_global::<SharedPerformanceMonitor>() {
            return shared.0.clone();
        }
        let monitor = cx.new(|cx| {
            let sample = Rc::new(Cell::new(None));
            let latest = sample.clone();
            let task = sysinfo::IS_SUPPORTED_SYSTEM.then(|| {
                cx.spawn(async move |this, cx| {
                    let pid = sysinfo::get_current_pid().ok();
                    let mut system = System::new();
                    loop {
                        let (refreshed, sample) = cx
                            .background_executor()
                            .spawn(async move {
                                let sample = refresh_performance(&mut system, pid);
                                (system, sample)
                            })
                            .await;
                        system = refreshed;
                        latest.set(Some(sample));
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                        cx.background_executor()
                            .timer(PERFORMANCE_SAMPLE_INTERVAL)
                            .await;
                    }
                })
            });
            Self {
                sample,
                _task: task,
            }
        });
        cx.set_global(SharedPerformanceMonitor(monitor.clone()));
        monitor
    }
}

pub(super) fn uses_performance_samples(bars: &ShellBarsSettings) -> bool {
    [
        ShellBarModule::AppCpu,
        ShellBarModule::AppMemory,
        ShellBarModule::SystemCpu,
        ShellBarModule::SystemMemory,
    ]
    .iter()
    .any(|module| bars.contains(module))
}

#[derive(Default)]
pub(super) struct PerformanceMonitorState {
    sample: Option<Rc<Cell<Option<PerformanceSample>>>>,
    subscription: Option<Subscription>,
}

impl PerformanceMonitorState {
    pub(super) fn attach(&mut self, window: &Window, cx: &mut Context<WorkbenchView>) {
        if self.subscription.is_some() {
            return;
        }
        let monitor = PerformanceMonitor::shared(cx);
        let handle = window.window_handle();
        let subscription = cx.observe(&monitor, move |view, _, cx| {
            if uses_performance_samples(&view.app_settings.bars) {
                // Refresh only this window, not the owner's settings and editor windows.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.refresh());
                });
            }
        });
        self.sample = Some(monitor.read(cx).sample.clone());
        self.subscription = Some(subscription);
    }
}

impl WorkbenchView {
    pub fn visible_performance_info(&self) -> Option<PerformanceInfo> {
        let sample = self
            .performance
            .sample
            .as_ref()
            .and_then(|sample| sample.get());
        let application = Some({
            let projects = self.workspace.opened_projects();
            let project_count = projects.len();
            let terminal_count = self.terminal.terminal_panes.len();
            let terminal_tab_count = projects
                .iter()
                .map(|project| project.layout.tabs.len())
                .sum::<usize>();
            let editor_tab_count = projects
                .iter()
                .filter_map(|project| {
                    self.project
                        .project_editor_runtime
                        .workspace()
                        .session(&project.id)
                })
                .map(|session| session.file_ids().len())
                .sum::<usize>();
            let editor_count = projects
                .iter()
                .map(|project| {
                    self.project
                        .project_editor_runtime
                        .documents_for_project(&project.id)
                        .count()
                })
                .sum::<usize>();
            let cpu = sample.and_then(|sample| sample.application).map_or_else(
                || "—".to_string(),
                |sample| format!("{:.1}%", sample.cpu_percent),
            );
            let memory = sample.and_then(|sample| sample.application).map_or_else(
                || "—".to_string(),
                |sample| format!("{:.1} MiB", sample.memory_bytes as f64 / MEBIBYTE_BYTES),
            );

            ApplicationPerformanceInfo {
                projects: performance_metric(
                    self.ui_text.get(UiTextKey::PerformanceProjects),
                    project_count.to_string(),
                ),
                terminals: performance_metric(
                    self.ui_text.get(UiTextKey::PerformanceTerminals),
                    terminal_count.to_string(),
                ),
                tabs: performance_metric(
                    self.ui_text.get(UiTextKey::PerformanceTabs),
                    (terminal_tab_count + editor_tab_count).to_string(),
                ),
                editors: performance_metric(
                    self.ui_text.get(UiTextKey::PerformanceEditors),
                    editor_count.to_string(),
                ),
                cpu: performance_metric(self.ui_text.get(UiTextKey::PerformanceCpu), cpu),
                memory: performance_metric(self.ui_text.get(UiTextKey::PerformanceMemory), memory),
            }
        });
        let system = sample.and_then(|sample| sample.system).map(|sample| {
            let cpu = format!("{:.1}%", sample.cpu_percent);
            let memory = format!("{:.1}%", sample.memory_percent);

            SystemPerformanceInfo {
                cpu: performance_metric(self.ui_text.get(UiTextKey::PerformanceSystemCpu), cpu),
                memory: performance_metric(
                    self.ui_text.get(UiTextKey::PerformanceSystemMemory),
                    memory,
                ),
            }
        });

        Some(PerformanceInfo {
            application,
            system,
        })
    }
}

fn performance_metric(label: &'static str, value: String) -> PerformanceMetricInfo {
    PerformanceMetricInfo {
        tooltip: format!("{label}: {value}"),
        value,
    }
}

fn refresh_performance(system: &mut System, pid: Option<Pid>) -> PerformanceSample {
    let application = pid.and_then(|pid| {
        let pids = [pid];
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        system
            .process(pid)
            .map(|process| ApplicationPerformanceSample {
                cpu_percent: process.cpu_usage(),
                memory_bytes: process.memory(),
            })
    });

    let system_sample = {
        system.refresh_cpu_usage();
        system.refresh_memory();
        let total_memory = system.total_memory();
        let memory_percent = if total_memory == 0 {
            0.0
        } else {
            (system.used_memory() as f64 / total_memory as f64 * 100.0) as f32
        };
        SystemPerformanceSample {
            cpu_percent: system.global_cpu_usage(),
            memory_percent,
        }
    };

    PerformanceSample {
        application,
        system: Some(system_sample),
    }
}
