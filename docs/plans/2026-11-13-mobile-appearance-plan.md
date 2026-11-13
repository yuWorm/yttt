# 手机端外观设置、编辑器语法高亮、收起键盘：计划

- 日期：2026-11-13
- 分支：`feat/mobile-appearance`（worktree `.worktrees/mobile-appearance`）
- 依据：用户反馈（iOS 真机）：tab 条右侧“全部 Tab”按钮的背景把标题栏割裂；设置里缺字体、字号、终端主题、编辑器高亮；终端唤出系统键盘后没有办法收起。

## 1. Tab 条按钮（iOS）

`TabStrip` 的 `safeAreaInset` 按钮去掉自己的 `.background(.bar)` 与左侧 `Divider`，和整条 tab 栏用同一个背景。`safeAreaInset` 已让 tab 不会滚到按钮下面。

## 2. 收起键盘（iOS）

SwiftTerm 键盘条上的 ⌨ 只切到它自己的功能键键盘，不收起。接管时：

- 系统键盘弹出期间，导航栏右侧出现“收起键盘”（`keyboard.chevron.compact.down`）；
- 在终端上向下轻扫也收起；
- 再点终端，键盘回来。

Android 用系统返回手势收起输入法，不加。

## 3. 外观设置（两端）

设置页新增“外观”一节，两行：“终端”“编辑器”，各推入一页，顶部是实时预览。

**终端**
- 字体：系统等宽（iOS SF Mono / Android Droid Sans Mono）、JetBrains Mono（随应用附带，OFL-1.1，Regular + Bold）；iOS 另有 Menlo、Courier New。
- 字号：8–20，默认同现在（iOS 11、Android 现值）。
- 主题：跟随系统（默认，暗色 / 亮色）、暗色、亮色、Solarized 暗、Solarized 亮、One Dark、Nord。

**编辑器**
- 语法高亮：按扩展名选语言；
  - iOS 加 `TreeSitterLanguages`（Runestone 作者的配套库，MIT，固定版本）；
  - Android 加 sora-editor 的 `language-textmate`（与 editor 同版本），语法文件取自 VS Code 内置扩展（MIT），只带常用语言：JS/TS、JSON、Python、Rust、Go、Swift、Java、C/C++、Markdown、YAML、Shell、HTML、CSS、XML 等。
- 主题：同终端的列表，默认跟随系统。
- 字体、字号（10–24，默认 14）、自动换行、行号。

**配色只有一份**：在核心里（`yttt-mobile-core::appearance`）定义每个主题的背景、前景、光标、选区、16 色 ANSI，以及由它们得到的语法高亮颜色（关键字、字符串、注释、数字、函数、类型、常量、运算符、标点）；经 FFI 导出，两端按 id 取用。

**存储**：iOS `UserDefaults`（`@AppStorage`），Android `SharedPreferences` + `StateFlow`；改了立即作用于打开着的终端和编辑器。

## 4. 测试

- Rust：主题 id 唯一；每个主题 16 色齐全；前景与背景、注释与背景的对比度达标；“跟随系统”按暗/亮选对主题。
- 两端单测：设置的默认值、越界字号收回范围、未知主题 id 回到默认、扩展名到语言的映射。
- 模拟器：终端与编辑器在各主题下截图；字号改动后终端重新排版、接管时网格大小随之变化；iOS 键盘收起与再唤出。
- 正式版：`check-release.sh`。

## 不做

- 自定义主题、导入主题文件。
- 编辑器的代码补全、括号匹配之外的语言功能。
