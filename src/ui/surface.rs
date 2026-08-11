#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkbenchSurface {
    #[default]
    Workspace,
    Editor,
    Terminal,
    Projects,
    ProjectTree,
    Settings,
    Palette,
    GitDiff,
    Dialog,
}

impl WorkbenchSurface {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Editor => "editor",
            Self::Terminal => "terminal",
            Self::Projects => "projects",
            Self::ProjectTree => "tree",
            Self::Settings => "settings",
            Self::Palette => "palette",
            Self::GitDiff => "git-diff",
            Self::Dialog => "dialog",
        }
    }
}
