#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    English,
    Chinese,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiTextKey {
    AppName,
    EmptySubtitle,
    EmptySidebarNote,
    OpenDirectory,
    OpenRecent,
    CommandPalette,
    NewTab,
    NoTerminalTabs,
    Projects,
    Tabs,
    Lazy,
    Started,
    Active,
    NoResults,
    TypeToFilter,
    CloseProjectTitle,
    CloseProjectBody,
    Cancel,
    CloseProjectAction,
    RenameTabTitle,
    RenameTabAction,
    RenameTabHint,
}

#[derive(Clone, Copy, Debug)]
pub struct UiText {
    locale: Locale,
}

impl UiText {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub fn english() -> Self {
        Self::new(Locale::English)
    }

    pub fn get(&self, key: UiTextKey) -> &'static str {
        match self.locale {
            Locale::English => english(key),
            Locale::Chinese => chinese(key),
        }
    }
}

fn english(key: UiTextKey) -> &'static str {
    match key {
        UiTextKey::AppName => "yttt",
        UiTextKey::EmptySubtitle => "Open a directory or choose a recent project.",
        UiTextKey::EmptySidebarNote => "Sidebar shows opened projects only.",
        UiTextKey::OpenDirectory => "Open Directory",
        UiTextKey::OpenRecent => "Open Recent",
        UiTextKey::CommandPalette => "Command Palette",
        UiTextKey::NewTab => "New Tab",
        UiTextKey::NoTerminalTabs => "No terminal tabs",
        UiTextKey::Projects => "Projects",
        UiTextKey::Tabs => "Tabs",
        UiTextKey::Lazy => "lazy",
        UiTextKey::Started => "started",
        UiTextKey::Active => "active",
        UiTextKey::NoResults => "No results",
        UiTextKey::TypeToFilter => "Type to filter",
        UiTextKey::CloseProjectTitle => "Close project?",
        UiTextKey::CloseProjectBody => "Running terminal processes will be stopped.",
        UiTextKey::Cancel => "Cancel",
        UiTextKey::CloseProjectAction => "Close Project",
        UiTextKey::RenameTabTitle => "Rename tab",
        UiTextKey::RenameTabAction => "Rename",
        UiTextKey::RenameTabHint => "Enter to rename, Escape to cancel",
    }
}

fn chinese(key: UiTextKey) -> &'static str {
    match key {
        UiTextKey::AppName => "yttt",
        UiTextKey::EmptySubtitle => "打开目录，或从最近项目中选择。",
        UiTextKey::EmptySidebarNote => "侧边栏只显示已打开的项目。",
        UiTextKey::OpenDirectory => "打开目录",
        UiTextKey::OpenRecent => "打开最近项目",
        UiTextKey::CommandPalette => "命令面板",
        UiTextKey::NewTab => "新建标签页",
        UiTextKey::NoTerminalTabs => "暂无终端标签页",
        UiTextKey::Projects => "项目",
        UiTextKey::Tabs => "标签页",
        UiTextKey::Lazy => "未启动",
        UiTextKey::Started => "已启动",
        UiTextKey::Active => "当前",
        UiTextKey::NoResults => "无结果",
        UiTextKey::TypeToFilter => "输入以筛选",
        UiTextKey::CloseProjectTitle => "关闭项目？",
        UiTextKey::CloseProjectBody => "正在运行的终端进程会被停止。",
        UiTextKey::Cancel => "取消",
        UiTextKey::CloseProjectAction => "关闭项目",
        UiTextKey::RenameTabTitle => "重命名标签页",
        UiTextKey::RenameTabAction => "重命名",
        UiTextKey::RenameTabHint => "回车重命名，Esc 取消",
    }
}
