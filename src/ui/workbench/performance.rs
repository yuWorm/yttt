use std::time::Duration;

use gpui::{Context, Task};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::WorkbenchView;
use crate::ui::{
    i18n::UiTextKey,
    workbench::shell::titlebar::{
        TitlebarApplicationPerformanceInfo, TitlebarMetricInfo, TitlebarPerformanceInfo,
        TitlebarSystemPerformanceInfo,
    },
};

const PERFORMANCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const MEBIBYTE_BYTES: f64 = 1024.0 * 1024.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PerformanceCollectionMode {
    application: bool,
    system: bool,
}

impl PerformanceCollectionMode {
    fn is_empty(self) -> bool {
        !self.application && !self.system
    }
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

#[derive(Default)]
pub(super) struct PerformanceMonitorState {
    sample: Option<PerformanceSample>,
    task: Option<Task<()>>,
    mode: PerformanceCollectionMode,
}

impl WorkbenchView {
    fn performance_collection_mode(&self) -> PerformanceCollectionMode {
        PerformanceCollectionMode {
            application: self.app_settings.general.performance_metrics_enabled,
            system: self.app_settings.general.system_performance_metrics_enabled,
        }
    }

    pub(crate) fn sync_performance_monitoring(&mut self, cx: &mut Context<Self>) {
        let mode = self.performance_collection_mode();
        if mode.is_empty() || !sysinfo::IS_SUPPORTED_SYSTEM {
            self.performance.task.take();
            self.performance.sample = None;
            self.performance.mode = mode;
            return;
        }
        if self.performance.task.is_some() && self.performance.mode == mode {
            return;
        }

        self.performance.task.take();
        self.performance.sample = None;
        self.performance.mode = mode;
        let pid = mode
            .application
            .then(sysinfo::get_current_pid)
            .and_then(Result::ok);

        self.performance.task = Some(cx.spawn(async move |this, cx| {
            let initial_refresh = cx.background_executor().spawn(async move {
                let mut system = System::new();
                let sample = refresh_performance(&mut system, pid, mode);
                (system, sample)
            });
            let (mut system, initial_sample) = initial_refresh.await;
            let initial_applied = this.update(cx, |view, cx| {
                if view.performance_collection_mode() != mode {
                    return false;
                }
                view.performance.sample = Some(initial_sample);
                cx.notify();
                true
            });
            if !matches!(initial_applied, Ok(true)) {
                return;
            }

            loop {
                cx.background_executor()
                    .timer(PERFORMANCE_SAMPLE_INTERVAL)
                    .await;
                let refresh = cx.background_executor().spawn(async move {
                    let sample = refresh_performance(&mut system, pid, mode);
                    (system, sample)
                });
                let (refreshed_system, sample) = refresh.await;
                system = refreshed_system;
                let applied = this.update(cx, |view, cx| {
                    if view.performance_collection_mode() != mode {
                        return false;
                    }
                    view.performance.sample = Some(sample);
                    cx.notify();
                    true
                });
                if !matches!(applied, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub fn visible_titlebar_performance(&self) -> Option<TitlebarPerformanceInfo> {
        let application = self
            .app_settings
            .general
            .performance_metrics_enabled
            .then(|| {
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
                let cpu = self
                    .performance
                    .sample
                    .and_then(|sample| sample.application)
                    .map_or_else(
                        || "—".to_string(),
                        |sample| format!("{:.1}%", sample.cpu_percent),
                    );
                let memory = self
                    .performance
                    .sample
                    .and_then(|sample| sample.application)
                    .map_or_else(
                        || "—".to_string(),
                        |sample| format!("{:.1} MiB", sample.memory_bytes as f64 / MEBIBYTE_BYTES),
                    );

                TitlebarApplicationPerformanceInfo {
                    projects: titlebar_metric(
                        self.ui_text.get(UiTextKey::PerformanceProjects),
                        project_count.to_string(),
                    ),
                    terminals: titlebar_metric(
                        self.ui_text.get(UiTextKey::PerformanceTerminals),
                        terminal_count.to_string(),
                    ),
                    tabs: titlebar_metric(
                        self.ui_text.get(UiTextKey::PerformanceTabs),
                        (terminal_tab_count + editor_tab_count).to_string(),
                    ),
                    editors: titlebar_metric(
                        self.ui_text.get(UiTextKey::PerformanceEditors),
                        editor_count.to_string(),
                    ),
                    cpu: titlebar_metric(self.ui_text.get(UiTextKey::PerformanceCpu), cpu),
                    memory: titlebar_metric(self.ui_text.get(UiTextKey::PerformanceMemory), memory),
                }
            });
        let system = self
            .app_settings
            .general
            .system_performance_metrics_enabled
            .then(|| {
                let cpu = self
                    .performance
                    .sample
                    .and_then(|sample| sample.system)
                    .map_or_else(
                        || "—".to_string(),
                        |sample| format!("{:.1}%", sample.cpu_percent),
                    );
                let memory = self
                    .performance
                    .sample
                    .and_then(|sample| sample.system)
                    .map_or_else(
                        || "—".to_string(),
                        |sample| format!("{:.1}%", sample.memory_percent),
                    );

                TitlebarSystemPerformanceInfo {
                    cpu: titlebar_metric(self.ui_text.get(UiTextKey::PerformanceSystemCpu), cpu),
                    memory: titlebar_metric(
                        self.ui_text.get(UiTextKey::PerformanceSystemMemory),
                        memory,
                    ),
                }
            });

        if application.is_none() && system.is_none() {
            None
        } else {
            Some(TitlebarPerformanceInfo {
                application,
                system,
            })
        }
    }
}

fn titlebar_metric(label: &'static str, value: String) -> TitlebarMetricInfo {
    TitlebarMetricInfo {
        tooltip: format!("{label}: {value}"),
        value,
    }
}

fn refresh_performance(
    system: &mut System,
    pid: Option<Pid>,
    mode: PerformanceCollectionMode,
) -> PerformanceSample {
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

    let system_sample = mode.system.then(|| {
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
    });

    PerformanceSample {
        application,
        system: system_sample,
    }
}
