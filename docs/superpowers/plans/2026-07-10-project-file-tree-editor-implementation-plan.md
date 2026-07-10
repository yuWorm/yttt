# Project File Tree and Editor Implementation Plan

> **For agentic workers:** REQUIRED: Follow repository `AGENTS.md`. Execute this plan inline in the current session with `@superpowers:executing-plans`; do not use subagent-driven development. Use each checkbox for progress tracking.

**Goal:** Add a right-side project file tree, unified terminal/file tabs, safe project-file editing and saving, resizable sidebars, and effective editor settings without changing the persistence meaning of terminal layouts.

**Architecture:** Keep `Workspace`, `ProjectLayout.tabs`, and terminal runtime state terminal-only. Add a UI-owned per-project work-item/session layer plus focused file-tree, file-I/O, and editor-document modules; the tab bar merges terminal and file snapshots while `RootView` coordinates GPUI entities and background work. All filesystem work is performed off the UI thread, and every asynchronous result is guarded by project/document generations.

**Tech Stack:** Rust 2024, GPUI 0.2.2, gpui-component 0.5.1 (`Input`, `Tree`, `TreeState`), serde/TOML, UUID-backed atomic temp files, existing yttt theme/i18n/command infrastructure.

**Required skills:** `@superpowers:test-driven-development`, `@gpui`, `@gpui-component`, `@superpowers:verification-before-completion`.

**Design spec:** `docs/superpowers/specs/2026-07-10-project-file-tree-editor-design.md`

**Baseline:** `cargo test` on 2026-07-10: 336 passed, 2 ignored, 0 failed. The existing `block v0.1.6` future-incompatibility warning is pre-existing.

**Version-specific decisions:**

- Render editor font family, size, and line height through `Input` style refinement; `gpui-component` applies the style to `window.text_style()` before `TextElement` prepaint, so the existing `InputState` and undo history survive.
- Update soft-wrap and line numbers through `InputState::set_soft_wrap` and `InputState::set_line_number`.
- Apply `tab_size` at `InputState` construction with `TabSize`; gpui-component 0.5.1 has no runtime setter, so existing documents pick it up only after reopening, as approved in the spec.
- Use `gpui_component::tree::{Tree, TreeState, TreeItem}` as the virtualized view adapter, with yttt's pure path-based tree as the source of truth.
- Do not use `gpui_component::ResizableState`: it has no public programmatic size setter and proportionally rescales all panels. Extend yttt's local sidebar primitive so collapse/hide and per-project width restoration remain deterministic.

---

## File map

### New focused modules

- `src/ui/editor/file_io.rs` — bounded UTF-8 reads, path containment, disk fingerprints, conflict detection, and atomic writes.
- `src/ui/editor/document.rs` — pure document model plus the GPUI `ProjectEditorDocument` entity that owns one `InputState`.
- `src/ui/editor/workspace.rs` — `WorkItemId`, per-project file-tab/session state, and cross-project runtime registry.
- `src/ui/project_tree/mod.rs` — project-tree exports.
- `src/ui/project_tree/fs.rs` — direct-directory scans and safe entry metadata.
- `src/ui/project_tree/state.rs` — lazy path-based tree state and visible-row flattening.
- `src/ui/project_tree/view.rs` — gpui-component `TreeState` adapter and user events.
- `tests/project_file_io.rs` — file safety, conflict, and atomic-write integration tests.
- `tests/project_tree.rs` — scanning, filtering, lazy-state, sorting, and symlink tests.
- `tests/editor_workspace.rs` — work-item ordering, selection, deduplication, and project isolation.
- `tests/editor_document.rs` — save-baseline/generation model tests.

### Existing modules with targeted changes

- `src/config/settings.rs` / `tests/settings_config.rs` — editor and project-panel settings schema/validation.
- `src/ui/editor/state.rs`, `src/ui/editor/view.rs`, `src/ui/editor/mod.rs`, `tests/editor.rs` — tab size and saved-baseline APIs.
- `src/ui/primitives/sidebar.rs`, `src/ui/sidebar.rs`, `tests/ui_shell.rs` — shared left/right resize rules and width-aware rendering.
- `src/runtime/git_status.rs`, `tests/ui_state.rs` — per-path Git decoration.
- `src/ui/tabs.rs`, `tests/ui_state.rs`, `tests/ui_shell.rs` — unified work-item tab snapshots and fixed file-tree toggle.
- `src/commands/mod.rs`, `src/config/keybindings.rs`, `src/ui/actions.rs`, `src/ui/interaction/key_dispatch.rs`, `tests/commands_keybindings.rs` — file/project-panel commands and context-aware dispatch.
- `src/palette/mod.rs`, `tests/palette.rs` — file entries in the current-project tab palette.
- `src/ui/root.rs`, `src/ui/root/dialogs.rs`, `src/ui/root/helpers.rs`, `src/ui/app.rs`, `tests/ui_state.rs` — lifecycle, async orchestration, focus, save/conflict/close flows, and window-close protection.
- `src/ui/settings.rs`, `src/ui/font_options.rs`, `src/ui/i18n.rs`, `tests/ui_i18n.rs` — settings UI and localized text.
- `README.md`, `docs/usage.md` — product direction, settings reference, limits, and smoke checklist.

---

## Chunk 1: Configuration and pure foundations

### Task 1: Add validated editor and project-panel settings

**Files:**

- Modify: `src/config/settings.rs:8-145, 360-403`
- Modify: `tests/settings_config.rs`

- [ ] **Step 1: Write failing default and round-trip tests**

Add tests that require these public values:

```rust
use yttt::config::settings::{AppSettings, EditorAutosave};

#[test]
fn editor_and_project_panel_defaults_match_the_design() {
    let settings = AppSettings::default();
    assert_eq!(settings.editor.font_family, "");
    assert_eq!(settings.editor.font_size, 14.0);
    assert_eq!(settings.editor.line_height, 1.4);
    assert_eq!(settings.editor.tab_size, 4);
    assert!(!settings.editor.soft_wrap);
    assert!(settings.editor.line_numbers);
    assert_eq!(settings.editor.autosave, EditorAutosave::Off);
    assert_eq!(settings.editor.autosave_delay_ms, 1000);
    assert!(settings.project_panel.default_open);
    assert!(!settings.project_panel.show_hidden);
    assert_eq!(settings.project_panel.width, 280.0);
    assert_eq!(settings.project_panel.project_sidebar_width, 216.0);
}
```

Also round-trip every `EditorAutosave` variant through TOML and persist non-default editor/panel settings through `save_settings` + `load_or_create_settings`.

- [ ] **Step 2: Run the new tests and verify RED**

Run: `cargo test --test settings_config editor_and_project_panel_defaults_match_the_design -- --exact`

Expected: compile failure because `EditorAutosave`, the editor display fields, and `project_panel` do not exist.

- [ ] **Step 3: Add the schema and defaults**

Implement the following shape while retaining the existing language/LSP fields:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAutosave {
    #[default]
    Off,
    OnFocusChange,
    AfterDelay,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectPanelSettings {
    pub default_open: bool,
    pub show_hidden: bool,
    pub width: f32,
    pub project_sidebar_width: f32,
}
```

Add the approved editor fields to `EditorSettings`, add `project_panel` to `AppSettings`, and use the spec defaults.

- [ ] **Step 4: Write failing invalid-value tests**

Cover `font_size <= 0`, `line_height < 1`, `tab_size` outside `1..=16`, zero autosave delay, non-finite/sidebar out-of-range widths, and an invalid autosave enum. Assert editor values fall back to defaults, widths clamp to their allowed range, and warnings identify the exact section/field without resetting unrelated settings sections.

- [ ] **Step 5: Run the validation tests and verify RED**

Run: `cargo test --test settings_config invalid_editor_and_project_panel_values_are_normalized -- --exact`

Expected: FAIL because validation and project-panel warnings are missing.

- [ ] **Step 6: Implement normalization without hiding warnings**

Add `InvalidProjectPanelValue { field }` to `SettingsLoadWarning`. Normalize the raw `editor.autosave` TOML value before `try_into::<AppSettings>()`, following `normalize_general_settings`, so an invalid enum becomes `off`, emits `InvalidEditorValue { field: "autosave" }`, and does not make deserialization reset the whole application settings object. Normalize the remaining editor fields in `validate_settings`; trim `font_family`; use finite/range checks; clamp `project_sidebar_width` to `160..=420` and `width` to `200..=520`.

- [ ] **Step 7: Run settings tests**

Run: `cargo test --test settings_config`

Expected: all settings tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/config/settings.rs tests/settings_config.rs
git commit -m "feat: add editor and project panel settings"
```

### Task 2: Extend the editor core for project-file configuration and async-save baselines

**Files:**

- Modify: `src/ui/editor/state.rs`
- Modify: `src/ui/editor/view.rs`
- Modify: `src/ui/editor/mod.rs`
- Modify: `tests/editor.rs`

- [ ] **Step 1: Write failing editor-core tests**

Add tests for configured tab width and for saving an older captured value while newer text remains dirty:

```rust
#[test]
fn marking_captured_value_saved_keeps_newer_edits_dirty() {
    let mut editor = project_editor("old");
    editor.set_value("first edit");
    let captured = editor.value().to_string();
    editor.set_value("newer edit");
    editor.mark_value_saved(captured);
    assert!(editor.is_dirty());
    assert_eq!(editor.value(), "newer edit");
}
```

Also test `replace_from_disk` updates both current and saved text and clears stale errors/diagnostics.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test editor marking_captured_value_saved_keeps_newer_edits_dirty -- --exact`

Expected: compile failure because `mark_value_saved` does not exist.

- [ ] **Step 3: Implement the minimal state API**

Add `tab_size: usize` and `with_tab_size` to `CodeEditorConfig`; add `saved_value()`, `mark_value_saved(value)`, and `replace_from_disk(value)` to `CodeEditorState`. `mark_saved()` should delegate to `mark_value_saved(self.value.clone())`.

- [ ] **Step 4: Apply tab size when constructing InputState**

Update `code_editor_input_state` with:

```rust
.tab_size(gpui_component::input::TabSize {
    tab_size: editor.config().tab_size(),
    hard_tabs: false,
})
```

Do not add a fake runtime setter or reconstruct `InputState`.

- [ ] **Step 5: Run editor tests**

Run: `cargo test --test editor`

Expected: all editor tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui/editor/state.rs src/ui/editor/view.rs src/ui/editor/mod.rs tests/editor.rs
git commit -m "feat: prepare editor state for project files"
```

### Task 3: Implement bounded project-file reads, fingerprints, conflicts, and atomic writes

**Files:**

- Create: `src/ui/editor/file_io.rs`
- Modify: `src/ui/editor/mod.rs`
- Create: `tests/project_file_io.rs`

- [ ] **Step 1: Write failing safe-read tests**

Cover:

- a normal UTF-8 file returns its canonical path, text, and fingerprint;
- absolute paths and `..` paths are rejected;
- a symlink escaping the project is rejected under `#[cfg(unix)]`;
- NUL-containing/non-UTF-8 files and files larger than `MAX_PROJECT_FILE_BYTES` are rejected.

Use this public contract:

```rust
pub const MAX_PROJECT_FILE_BYTES: u64 = 10 * 1024 * 1024;

pub struct LoadedProjectFile {
    pub canonical_path: PathBuf,
    pub relative_path: PathBuf,
    pub text: String,
    pub fingerprint: DiskFingerprint,
}

pub fn read_project_file(root: &Path, relative_path: &Path)
    -> Result<LoadedProjectFile, ProjectFileIoError>;
```

- [ ] **Step 2: Run safe-read tests and verify RED**

Run: `cargo test --test project_file_io read_project_file_rejects_paths_outside_the_project -- --exact`

Expected: compile failure because the module/API does not exist.

- [ ] **Step 3: Implement path and content validation**

Reject `Component::ParentDir`, `RootDir`, and platform prefixes before joining. Canonicalize the project root and existing target, require `canonical_target.starts_with(canonical_root)`, inspect metadata before allocation, read at most the approved limit, reject NUL bytes, and convert with `String::from_utf8`.

Define a cloneable/equatable `DiskFingerprint` containing existence, byte length, modified time, and a runtime `DefaultHasher` content hash.

- [ ] **Step 4: Write failing conflict and atomic-write tests**

Require this behavior:

```rust
pub enum SaveMode<'a> {
    Check(&'a DiskFingerprint),
    Force,
}

pub enum SaveProjectFileOutcome {
    Saved(DiskFingerprint),
    Conflict(CurrentDiskState),
}
```

Test normal save, external same-length modification, external deletion, forced recreation, permission preservation on Unix, and an injected rename failure that leaves the old target unchanged.

- [ ] **Step 5: Run write tests and verify RED**

Run: `cargo test --test project_file_io save_project_file_reports_external_conflict -- --exact`

Expected: compile failure because save APIs do not exist.

- [ ] **Step 6: Implement conflict-aware atomic writes**

Re-read the current fingerprint before writing. Use a UUID-named `create_new` temp file in the destination directory, write all bytes, `sync_all`, copy original permissions when present, then rename. Add a small internal filesystem trait/test double so rename-failure preservation is tested without destructive tricks. Always attempt temp cleanup after failure.

- [ ] **Step 7: Run file-I/O tests**

Run: `cargo test --test project_file_io`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/ui/editor/file_io.rs src/ui/editor/mod.rs tests/project_file_io.rs
git commit -m "feat: add safe project file persistence"
```

### Task 4: Build the lazy project-tree filesystem and state model

**Files:**

- Create: `src/ui/project_tree/mod.rs`
- Create: `src/ui/project_tree/fs.rs`
- Create: `src/ui/project_tree/state.rs`
- Modify: `src/ui/mod.rs`
- Create: `tests/project_tree.rs`

- [ ] **Step 1: Write failing directory-scan tests**

Test case-insensitive stable sorting with directories first, hidden-file filtering, empty directories, and symlink classification. Use direct-directory snapshots rather than recursive scans:

```rust
pub struct DirectorySnapshot {
    pub relative_directory: PathBuf,
    pub entries: Vec<ProjectTreeEntry>,
}

pub fn scan_project_directory(
    root: &Path,
    relative_directory: &Path,
    show_hidden: bool,
) -> Result<DirectorySnapshot, ProjectTreeFsError>;
```

- [ ] **Step 2: Run scan tests and verify RED**

Run: `cargo test --test project_tree scan_sorts_directories_before_files_case_insensitively -- --exact`

Expected: compile failure because `ui::project_tree` does not exist.

- [ ] **Step 3: Implement safe direct-directory scans**

Reuse the same relative-path containment principles as file I/O. Use `symlink_metadata`; represent `Directory`, `File`, `SymlinkFile`, and `SymlinkDirectory` explicitly; never traverse symlink directories.

- [ ] **Step 4: Write failing lazy-state tests**

Require a path-based state independent of row indices:

```rust
pub struct ProjectFileTree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
    directories: HashMap<PathBuf, DirectorySnapshot>,
    loading: HashSet<PathBuf>,
    errors: HashMap<PathBuf, String>,
    selected: Option<PathBuf>,
    generation: u64,
}
```

Test `request_expand` returns a load request for an unloaded directory, `apply_snapshot` exposes its children, collapse hides descendants without discarding snapshots, refresh keeps expansion for surviving paths, and errors remain local/retryable.

- [ ] **Step 5: Run state tests and verify RED**

Run: `cargo test --test project_tree expanding_an_unloaded_directory_requests_one_scan -- --exact`

Expected: FAIL because the state API is missing.

- [ ] **Step 6: Implement state and flattened visible rows**

Expose `visible_rows()` with stable depth, explicit kind, load state, selected state, and relative path. Increment generation on refresh/reset and require matching generation when applying async snapshots.

- [ ] **Step 7: Run project-tree tests**

Run: `cargo test --test project_tree`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/ui/project_tree src/ui/mod.rs tests/project_tree.rs
git commit -m "feat: add lazy project file tree model"
```

### Task 5: Add per-project work-item sessions

**Files:**

- Create: `src/ui/editor/workspace.rs`
- Modify: `src/ui/editor/mod.rs`
- Create: `tests/editor_workspace.rs`

- [ ] **Step 1: Write failing selection and deduplication tests**

Define stable IDs that do not enter `ProjectLayout` serialization:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WorkItemId {
    Terminal(String),
    File(DocumentId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId {
    pub project_id: ProjectId,
    pub canonical_path: PathBuf,
}
```

Test terminal tabs stay first, files append in open order, reopening the same canonical path activates without duplicating, and file selection never mutates the workspace's last selected terminal tab.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test editor_workspace opening_the_same_file_twice_reuses_one_work_item -- --exact`

Expected: compile failure because work-item sessions do not exist.

- [ ] **Step 3: Implement `ProjectWorkItemSession`**

Store ordered file IDs, active work item, activation history, `ProjectFileTree`, panel visibility, and width. Accept the current terminal IDs as an argument to `ordered_items`, `select_next`, `select_previous`, and `close_file`, so terminal configuration remains authoritative.

- [ ] **Step 4: Write failing project-isolation and close-neighbor tests**

Test two projects retain independent active files/tree state/panel width, and closing the active file chooses the right neighbor then left neighbor across the combined terminal+file order.

- [ ] **Step 5: Implement `ProjectEditorWorkspaceState` registry**

Add `HashMap<ProjectId, ProjectWorkItemSession>` with `open_project`, `close_project`, immutable/mutable session access, and no GPUI entities.

- [ ] **Step 6: Run session tests**

Run: `cargo test --test editor_workspace`

Expected: all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ui/editor/workspace.rs src/ui/editor/mod.rs tests/editor_workspace.rs
git commit -m "feat: add per-project work item sessions"
```

### Task 6: Generalize the sidebar primitive for directional resizing

**Files:**

- Modify: `src/ui/primitives/sidebar.rs`
- Modify: `src/ui/sidebar.rs`
- Modify: `tests/ui_shell.rs`

- [ ] **Step 1: Write failing width-direction tests**

```rust
#[test]
fn right_sidebar_grows_when_dragged_left() {
    assert_eq!(resize_sidebar_width(SidebarSide::Right, 280.0, -40.0, 200.0, 520.0), 320.0);
}

#[test]
fn sidebar_resize_clamps_at_both_bounds() { /* left and right cases */ }
```

Also assert collapsing/hiding changes the visible width without overwriting the stored expanded width.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_shell right_sidebar_grows_when_dragged_left -- --exact`

Expected: compile failure because `SidebarSide` and `resize_sidebar_width` do not exist.

- [ ] **Step 3: Implement the pure shared primitive**

Add `SidebarSide::{Left, Right}`, `SidebarWidthState`, and a pure delta/clamp helper. Extend `YtttSidebarStyle` with min/max/default widths and a 5px hit area while keeping the visible divider 1px.

- [ ] **Step 4: Make the left sidebar width-aware without wiring pointer events yet**

Change `project_sidebar` to receive an expanded width and use `collapsed_width` only when collapsed. Preserve existing visual and context-menu behavior.

- [ ] **Step 5: Run sidebar/UI tests**

Run: `cargo test --test ui_shell sidebar -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui/primitives/sidebar.rs src/ui/sidebar.rs tests/ui_shell.rs
git commit -m "feat: generalize sidebar sizing"
```

### Chunk 1 checkpoint

- [ ] Run: `cargo test --test settings_config --test editor --test project_file_io --test project_tree --test editor_workspace --test ui_shell`
- [ ] Expected: all selected tests PASS with only the pre-existing future-incompatibility warning.

---

## Chunk 2: GPUI components and workbench integration

### Task 7: Extend Git status with per-path decorations

**Files:**

- Modify: `src/runtime/git_status.rs`
- Modify: `tests/ui_state.rs:35-75`

- [ ] **Step 1: Write failing per-path parser tests**

Require modified, added, deleted, untracked, and rename destinations to be addressable by project-relative `PathBuf`:

```rust
#[test]
fn git_status_parser_records_file_tones() {
    let parsed = parse_git_status_porcelain(
        "## main\n M src/main.rs\nA  src/lib.rs\n D old.rs\n?? notes.txt\nR  old.txt -> new.txt\n",
    );
    assert_eq!(parsed.file_status(Path::new("src/main.rs")), Some(GitFileStatus::Modified));
    assert_eq!(parsed.file_status(Path::new("notes.txt")), Some(GitFileStatus::Untracked));
    assert_eq!(parsed.file_status(Path::new("new.txt")), Some(GitFileStatus::Modified));
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state git_status_parser_records_file_tones -- --exact`

Expected: compile failure because `GitFileStatus` and `file_status` do not exist.

- [ ] **Step 3: Implement the minimal path map**

Add `GitFileStatus::{Added, Modified, Deleted, Untracked}` and a `BTreeMap<PathBuf, GitFileStatus>` to `ProjectGitStatus`. Parse the path portion after the two status columns, choose the destination for `old -> new`, and keep the current summary's “count a dirty file once” behavior.

- [ ] **Step 4: Run Git/UI state tests**

Run: `cargo test --test ui_state git_status_ -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/runtime/git_status.rs tests/ui_state.rs
git commit -m "feat: expose per-file git status"
```

### Task 8: Adapt the pure file tree to gpui-component Tree

**Files:**

- Create: `src/ui/project_tree/view.rs`
- Modify: `src/ui/project_tree/mod.rs`
- Modify: `tests/project_tree.rs`
- Modify: `tests/ui_shell.rs`

- [ ] **Step 1: Write failing adapter tests**

Test that unloaded and empty directories still become folder-shaped `TreeItem`s, stable path IDs survive refresh, current-file selection maps back to the correct row, and Git tones appear in row snapshots.

The adapter contract should be explicit:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectTreeViewEvent {
    ToggleDirectory { path: PathBuf, expanded: bool },
    OpenFile(PathBuf),
    Refresh,
}

pub struct ProjectTreeView {
    tree: Entity<TreeState>,
    snapshot: ProjectTreeRenderSnapshot,
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test project_tree component_items_keep_empty_directories_expandable -- --exact`

Expected: compile failure because the view adapter does not exist.

- [ ] **Step 3: Implement snapshot-to-TreeItem conversion**

Convert path IDs to `SharedString`. Give unloaded/loading/empty directories a disabled synthetic child so gpui-component's `TreeItem::is_folder()` remains true. Keep synthetic IDs outside the real path namespace and never emit file-open events for them.

- [ ] **Step 4: Implement the GPUI entity and events**

Create `TreeState` once in `ProjectTreeView::new`. `sync` rebuilds items with `set_items`, then restores the selected index by path. Render with `tree(&self.tree, ...)` and a `ListItem` showing chevron/folder/file icon, basename, loading/error hint, and Git tone. Capture a weak `ProjectTreeView`; row clicks emit path-based events after gpui-component updates its internal row selection.

Avoid reading or updating `TreeState` recursively from its render callback; use the immutable snapshot, following the GPUI entity snapshot rule.

- [ ] **Step 5: Add a small GPUI event test**

Use `#[gpui::test]` only for entity/event behavior; keep rendering-independent conversion tests as normal Rust tests. Verify a file click emits `OpenFile(relative_path)` and a directory click emits the desired post-click expanded state.

- [ ] **Step 6: Run tree and UI tests**

Run: `cargo test --test project_tree --test ui_shell project_tree -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ui/project_tree/view.rs src/ui/project_tree/mod.rs tests/project_tree.rs tests/ui_shell.rs
git commit -m "feat: render project tree with gpui component"
```

### Task 9: Create one GPUI editor entity per open document

**Files:**

- Create: `src/ui/editor/document.rs`
- Modify: `src/ui/editor/mod.rs`
- Create: `tests/editor_document.rs`

- [ ] **Step 1: Write failing pure document-model tests**

Test edit generations, save snapshots, a completed older save leaving newer edits dirty, disk reload, and disk-version updates:

```rust
let request = model.begin_save();
model.on_input_changed("newer text");
model.finish_save(&request, new_fingerprint);
assert!(model.is_dirty());
assert_eq!(model.saved_value(), request.text);
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test editor_document completed_older_save_keeps_newer_edit_dirty -- --exact`

Expected: compile failure because `ProjectEditorModel` does not exist.

- [ ] **Step 3: Implement the pure model and event types**

Add:

```rust
pub struct EditorAppearance {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub soft_wrap: bool,
    pub line_numbers: bool,
}

pub enum ProjectEditorDocumentEvent {
    Changed { generation: u64 },
    Focused,
    Blurred,
}
```

The pure model owns `DocumentId`, `CodeEditorState`, `DiskFingerprint`, edit generation, and load/save state. It must not own GPUI tasks; cancellable autosave runtime belongs to `ProjectEditorDocument` (or the RootView runtime registry) so the model stays cloneable and deterministic in ordinary Rust tests.

- [ ] **Step 4: Write a failing GPUI InputEvent synchronization test**

Construct a `ProjectEditorDocument` in a GPUI test window, update its child `InputState`, and assert the document model becomes dirty and emits `Changed`.

- [ ] **Step 5: Implement `ProjectEditorDocument`**

In `new`, create the child input with `code_editor_input_state`, subscribe with `cx.subscribe_in`, and store the `Subscription`. On `InputEvent::Change`, read `state.value()`, update the pure model, emit `Changed`, and notify. Forward focus/blur events without nested same-entity updates.

Implement `Render` as:

```rust
let input = Input::new(&self.input)
    .h_full()
    .appearance(false)
    .text_size(px(self.appearance.font_size))
    .line_height(relative(self.appearance.line_height));
```

Conditionally add `.font_family(...)` only for a non-empty setting. `set_appearance` updates soft-wrap and line numbers through their public setters; it must not replace the child entity. `focus` delegates to `InputState::focus`.

- [ ] **Step 6: Run document/editor tests**

Run: `cargo test --test editor_document --test editor`

Expected: all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ui/editor/document.rs src/ui/editor/mod.rs tests/editor_document.rs
git commit -m "feat: add project editor document entity"
```

### Task 10: Render terminal and file work items in one tab bar

**Files:**

- Modify: `src/ui/tabs.rs`
- Modify: `src/ui/primitives/tabs.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_shell.rs`

- [ ] **Step 1: Write failing unified snapshot tests**

Add `WorkbenchTabItem`/`WorkbenchTabKind` tests that require terminal tabs first, file tabs in open order, file basename plus relative-path tooltip, selected state from `WorkItemId`, and a dirty marker that occupies the same slot as the hover close button.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state visible_work_item_tabs_merge_terminal_and_file_items -- --exact`

Expected: compile failure because unified tab snapshots do not exist.

- [ ] **Step 3: Refactor the renderer around passed snapshots**

Keep `visible_tab_items` as a terminal-only compatibility helper for existing tests, but make `project_tabs` render a passed `Vec<WorkbenchTabItem>`. Render `IconName::SquareTerminal` for terminal items and `IconName::File` for files. Preserve terminal status tones; render file dirty state with the warning/status color.

- [ ] **Step 4: Add the fixed project-tree toggle to the toolbar**

Extend the toolbar callback set with `on_toggle_project_tree` and `project_tree_open`. Put `FolderOpen`/`FolderClosed` after the existing new/split buttons, outside `project-tab-row.overflow_x_scroll()`. Use the selected/active surface style while open and state-specific tooltip text.

- [ ] **Step 5: Verify close propagation and density**

Keep `cx.stop_propagation()` in the caller's close callback so a close click cannot reselect the tab. Assert tab height remains 32px and the fixed toolbar never enters the scrolling container.

- [ ] **Step 6: Run tab tests**

Run: `cargo test --test ui_state visible_work_item_tabs -- --nocapture`

Run: `cargo test --test ui_shell tab_ -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ui/tabs.rs src/ui/primitives/tabs.rs tests/ui_state.rs tests/ui_shell.rs
git commit -m "feat: unify terminal and file tabs"
```

### Task 11: Add file and project-panel commands with editor-aware key dispatch

**Files:**

- Modify: `src/commands/mod.rs`
- Modify: `src/config/keybindings.rs`
- Modify: `src/ui/actions.rs`
- Modify: `src/ui/interaction/key_dispatch.rs`
- Modify: `src/palette/mod.rs`
- Modify: `tests/commands_keybindings.rs`
- Modify: `tests/palette.rs`

- [ ] **Step 1: Write failing command registration/binding tests**

Require `file.save`, `project_panel.toggle`, and `project_panel.refresh` in the registry. Add platform defaults for file save (`cmd-s` on macOS, `ctrl-s` elsewhere) and a toggle binding only if it does not conflict with an existing default; otherwise leave toggle command-palette accessible.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test commands_keybindings file_and_project_panel_commands_are_registered -- --exact`

Expected: compile failure because command IDs/actions are missing.

- [ ] **Step 3: Add command surface context without breaking pure workspace dispatch**

Introduce a commands-layer context independent of UI types:

```rust
pub enum ActiveSurface { None, Terminal, File }
pub struct CommandContext {
    pub has_selected_project: bool,
    pub active_surface: ActiveSurface,
}
```

Add `availability_for_context`; retain `availability(bool)` as a terminal-surface compatibility wrapper for existing callers/tests. File save is enabled only on `File`; pane and terminal rename/split commands only on `Terminal`; tab close/next/prev and project-panel commands work on either project surface.

- [ ] **Step 4: Write failing editor-owner dispatch tests**

Require `InputOwnerKind::Editor` to allow only the safe workspace commands (`FileSave`, tab navigation/close, panel toggle/refresh, palettes) while blocking pane movement and terminal input. Settings/layout modal editors must continue blocking project-file save through their higher-priority dialog state.

- [ ] **Step 5: Refactor `workspace_command_for_keystroke`**

Resolve the command first, then decide from `(owner, command)`. Keep terminal byte precedence when a terminal surface is active. Do not broadly treat every editor owner as workspace.

- [ ] **Step 6: Add unified tab-palette inputs**

Add a palette helper that accepts unified tab snapshots. Encode IDs with an unambiguous `terminal:` / `file:` prefix and expose file relative paths as subtitles. Keep the old terminal-only helper as a wrapper until root integration migrates.

- [ ] **Step 7: Run command/palette tests**

Run: `cargo test --test commands_keybindings --test palette`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/commands/mod.rs src/config/keybindings.rs src/ui/actions.rs src/ui/interaction/key_dispatch.rs src/palette/mod.rs tests/commands_keybindings.rs tests/palette.rs
git commit -m "feat: add editor workspace commands"
```

### Task 12: Add the runtime registry and per-project lifecycle to RootView

**Files:**

- Create: `src/ui/editor/runtime.rs`
- Modify: `src/ui/editor/mod.rs`
- Modify: `src/ui/root.rs:148-220, 274-390, 1658-1815`
- Modify: `tests/ui_state.rs`

- [ ] **Step 1: Write failing RootView lifecycle tests**

Test that opening a project creates one `ProjectWorkItemSession` seeded from the selected terminal and panel defaults; switching projects preserves both sessions; closing a confirmed project removes only its session/entities; fixtures initialized with a prebuilt `Workspace` also receive sessions.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state root_view_creates_project_work_item_session_on_open -- --exact`

Expected: compile failure because RootView exposes no project editor runtime.

- [ ] **Step 3: Implement the UI-owned runtime registry**

`ProjectEditorRuntime` should contain:

```rust
ProjectEditorWorkspaceState
HashMap<DocumentId, Entity<ProjectEditorDocument>>
HashMap<DocumentId, Subscription>
HashMap<ProjectId, Entity<ProjectTreeView>>
HashMap<ProjectId, Subscription>
HashMap<ProjectId, u64> // pending tree generations
HashMap<DocumentId, u64> // pending file generations
```

Provide focused insert/get/remove/snapshot APIs; keep fields private so `RootView` cannot mutate maps inconsistently.

- [ ] **Step 4: Integrate project open/select/close**

Initialize pure sessions in `with_workspace_and_config_paths` for all pre-existing projects. In `open_project_path`, create the session after `Workspace::open_project` succeeds. On project selection, switch only visibility. On final close, drop the project's tree/document entities and subscriptions after blockers are resolved.

- [ ] **Step 5: Run lifecycle tests**

Run: `cargo test --test ui_state root_view_ -- --nocapture`

Expected: existing root tests and new lifecycle tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui/editor/runtime.rs src/ui/editor/mod.rs src/ui/root.rs tests/ui_state.rs
git commit -m "feat: attach editor sessions to project lifecycle"
```

### Task 13: Wire unified selection, palettes, focus, and active-surface rendering

**Files:**

- Modify: `src/ui/root.rs:430-670, 1240-1580, 1700-1820, 2015-2085, 2540-2635, 3210-3590, 3718-3940`
- Modify: `src/ui/root/helpers.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/palette.rs`

- [ ] **Step 1: Write failing unified-selection tests**

Cover selecting a terminal work item, selecting a file work item, next/previous across the combined list, file entries in the tab palette, double-click rename only for terminal items, and pane/split commands remaining disabled while a file is active.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state root_view_tab_navigation_crosses_terminal_and_file_work_items -- --exact`

Expected: FAIL because root still delegates all tab navigation to `Workspace`.

- [ ] **Step 3: Add work-item dispatch helpers**

Implement `active_work_item`, `select_work_item`, `select_next_work_item`, and `select_previous_work_item`. Selecting `Terminal(id)` calls `Workspace::select_tab` and queues terminal focus; selecting `File(id)` leaves `OpenedProject.selected_tab_id` unchanged and queues editor focus.

Route `TabNext`, `TabPrev`, `TabClose`, and `TabRename` in `RootView::run_command` before the terminal-only workspace dispatcher. Build command palette availability from `CommandContext`.

- [ ] **Step 4: Migrate tab palette confirmation**

Decode the prefixed palette ID to `WorkItemId`, select it, and focus the correct surface. Pane palette returns no items while a file is active.

- [ ] **Step 5: Make input ownership surface-aware**

When an ordinary project file is active and no overlay/dialog is above it, register `InputOwnerKind::Editor` with a `editor.project_file:<project>:<path>` scope. Update `on_key_down` so raw editor arrows propagate to `InputState` instead of invoking pane focus fallbacks.

- [ ] **Step 6: Render the active surface and unified tabs**

Replace unconditional `active_terminal_split_view` with `active_work_item_view`: render the existing split tree for terminal items and the document entity for file items. Build `WorkbenchTabItem`s from terminal state plus document snapshots, pass them to `project_tabs`, and keep the file-tree toggle fixed at the toolbar end.

- [ ] **Step 7: Run selection/palette/input tests**

Run: `cargo test --test ui_state --test palette --test terminal_input`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/ui/root.rs src/ui/root/helpers.rs tests/ui_state.rs tests/palette.rs
git commit -m "feat: route unified work item selection"
```

### Task 14: Load directories/files asynchronously and render the right project panel

**Files:**

- Modify: `src/ui/root.rs`
- Modify: `src/ui/editor/runtime.rs`
- Modify: `src/ui/project_tree/view.rs`
- Modify: `src/ui/i18n.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_i18n.rs`

- [ ] **Step 1: Write failing state-level async result tests**

Test stale tree generations and stale file-open generations are ignored; opening the same pending file twice starts one load; failed reads create no document/Tab and preserve the previous active surface.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state stale_project_tree_result_does_not_replace_refreshed_state -- --exact`

Expected: compile failure because async load application helpers are missing.

- [ ] **Step 3: Implement tree entity creation/subscription**

Lazily create one `ProjectTreeView` entity per project when its panel is first shown, subscribe in `RootView`, and store the subscription in `ProjectEditorRuntime`. Translate view events to expand/collapse, direct-directory scan, file open, and refresh operations.

- [ ] **Step 4: Run filesystem work on GPUI's background executor**

Use the GPUI pattern:

```rust
let io_task = cx.background_spawn(async move { scan_project_directory(&root, &relative, hidden) });
cx.spawn_in(window, async move |this, cx| {
    let result = io_task.await;
    this.update_in(cx, |root, window, cx| {
        root.apply_tree_result(project_id, generation, result, window, cx);
    }).ok();
}).detach();
```

Use the same pattern for `read_project_file`. Check project/document generation before applying. Create `ProjectEditorDocument` only after a successful load, subscribe to its events, insert it in runtime, open/select its work item, then focus it.

- [ ] **Step 5: Render the right panel**

Add a header with selected project name and refresh button, the `ProjectTreeView`, local loading/error rows, theme tokens, and selected-file state. Show it only while the selected project's session says open. The toggle setting seeds new sessions; clicking the fixed toolbar button only changes current session state.

- [ ] **Step 6: Localize panel/file-load text**

Add English and Chinese keys for Files, show/hide files, refresh, loading, empty directory, directory error/retry, unsupported binary/encoding, over-size, and outside-project errors.

- [ ] **Step 7: Run tree/open/render tests**

Run: `cargo test --test project_tree --test editor_document --test ui_state --test ui_i18n`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/ui/root.rs src/ui/editor/runtime.rs src/ui/project_tree/view.rs src/ui/i18n.rs tests/ui_state.rs tests/ui_i18n.rs
git commit -m "feat: open project files from the file tree"
```

### Task 15: Wire pointer resizing for both sidebars and persist released widths

**Files:**

- Modify: `src/ui/root.rs:198-210, 251-256, 3170-3205, 3718-3940`
- Modify: `src/ui/sidebar.rs`
- Modify: `src/ui/primitives/sidebar.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_shell.rs`

- [ ] **Step 1: Write failing RootView drag tests**

Test left drag right increases the stored project sidebar width, right drag left increases only the selected project's panel width, both clamp, collapse/hide preserves stored expanded width, switching projects restores each right width, and mouse-up persists future defaults without changing already-open other projects.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state root_view_right_sidebar_drag_updates_only_selected_project -- --exact`

Expected: compile failure because sidebar drag state is missing.

- [ ] **Step 3: Add resize drag state and handles**

Add `ActiveSidebarResizeDrag { side, last_position }` alongside the existing split drag. Render a 5px absolute hit target with a 1px themed line between left/center and center/right. On mouse move, apply delta through the pure helper and update `last_position`; stop propagation only while a sidebar drag is active.

- [ ] **Step 4: Persist only on mouse-up**

Write `project_panel.project_sidebar_width` or `project_panel.width` through `save_settings` after release. On failure, retain the in-memory width and surface the existing error notification. Never persist 46px collapsed or 0px hidden widths.

- [ ] **Step 5: Run sidebar tests**

Run: `cargo test --test ui_state sidebar -- --nocapture`

Run: `cargo test --test ui_shell sidebar -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui/root.rs src/ui/sidebar.rs src/ui/primitives/sidebar.rs tests/ui_state.rs tests/ui_shell.rs
git commit -m "feat: make project sidebars resizable"
```

### Chunk 2 checkpoint

- [ ] Run: `cargo test --test commands_keybindings --test palette --test project_tree --test editor_document --test ui_shell --test ui_state`
- [ ] Expected: all selected tests PASS; manually inspect that no GPUI same-entity nested update was introduced in tree/document callbacks.

---


## Chunk 3: Persistence UX, settings UI, and verification

### Task 16: Implement manual save and external-change conflict resolution

**Files:**

- Modify: `src/ui/root.rs`
- Modify: `src/ui/root/dialogs.rs`
- Modify: `src/ui/editor/document.rs`
- Modify: `src/ui/editor/runtime.rs`
- Modify: `src/ui/i18n.rs`
- Modify: `tests/editor_document.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_i18n.rs`

- [ ] **Step 1: Write failing save lifecycle tests**

Cover:

- `FileSave` captures active text/fingerprint and enters saving state;
- successful save clears dirty only through the captured value;
- typing while save is in flight leaves newer text dirty;
- save failure leaves the document dirty and visible;
- clean document external change can reload;
- dirty document external change creates a pending conflict rather than writing.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state root_view_file_save_preserves_newer_in_flight_edit -- --exact`

Expected: FAIL because RootView has no file-save orchestration.

- [ ] **Step 3: Implement background save orchestration**

Add `save_document(document_id, SaveMode, continuation, window, cx)`. Read a `SaveRequest` from the document entity, run `save_project_file` on `cx.background_spawn`, and apply the result with `spawn_in`/`update_in`. Check the document still exists and the request generation/path still matches before updating.

On success, call `finish_save(request, fingerprint)` and run the continuation (none, close file, close project, or close window). On I/O error, retain dirty state and push a localized error notification.

- [ ] **Step 4: Add pending conflict state and dialog**

Model:

```rust
pub struct PendingFileConflict {
    document_id: DocumentId,
    request: SaveRequest,
    current_disk: CurrentDiskState,
    continuation: SaveContinuation,
}
```

Dialog actions:

- **Overwrite** — retry with `SaveMode::Force`.
- **Reload** — read disk in the background, call `replace_from_disk`, refresh language/highlighter if needed, and cancel any close continuation unless the user explicitly confirms discard.
- **Cancel** — keep memory and dirty state.
- For a missing file, label overwrite as **Recreate file**.

- [ ] **Step 5: Detect external changes at approved boundaries**

Before every manual/autosave, compare fingerprints. On document focus and project-tree refresh, schedule a lightweight background fingerprint/read check; auto-reload only clean documents. Never prompt repeatedly for the same disk version while one conflict is pending.

- [ ] **Step 6: Localize save/conflict UI**

Add English/Chinese keys for save, saving, saved, save failed, changed on disk, overwrite, reload, cancel, file deleted, and recreate.

- [ ] **Step 7: Run save/conflict tests**

Run: `cargo test --test project_file_io --test editor_document --test ui_state --test ui_i18n`

Expected: all tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src/ui/root.rs src/ui/root/dialogs.rs src/ui/editor/document.rs src/ui/editor/runtime.rs src/ui/i18n.rs tests/editor_document.rs tests/ui_state.rs tests/ui_i18n.rs
git commit -m "feat: save project files safely"
```

### Task 17: Implement focus-change and delayed autosave

**Files:**

- Modify: `src/ui/editor/document.rs`
- Modify: `src/ui/root.rs`
- Modify: `tests/editor_document.rs`
- Modify: `tests/ui_state.rs`

- [ ] **Step 1: Write failing autosave policy tests**

Test `Off` schedules nothing, `OnFocusChange` saves only dirty files on blur/work-item/project switch, and `AfterDelay` saves only the latest generation after the configured delay.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test editor_document delayed_autosave_discards_stale_generation -- --exact`

Expected: FAIL because autosave scheduling does not exist.

- [ ] **Step 3: Implement cancellable delayed scheduling**

On `Changed`, replace the document's stored task. The task uses `cx.background_executor().timer(Duration::from_millis(delay))`, then updates RootView only if document/project still exist and generation matches. Dropping/replacing the old `Task<()>` must cancel it.

- [ ] **Step 4: Implement focus-change triggers**

Handle `ProjectEditorDocumentEvent::Blurred`, `select_work_item`, and project selection. Call the same save pipeline as manual save. If a conflict or error appears, keep the original work-item switch—the data remains in memory and dirty—then surface the conflict/notification.

- [ ] **Step 5: Prevent overlapping autosaves**

If a save is already in flight, retain the newest requested generation and schedule one follow-up after completion. Do not spawn unbounded concurrent writes to one document.

- [ ] **Step 6: Run autosave tests**

Run: `cargo test --test editor_document autosave -- --nocapture`

Run: `cargo test --test ui_state autosave -- --nocapture`

Expected: all matching tests PASS deterministically with GPUI's test executor.

- [ ] **Step 7: Commit**

```bash
git add src/ui/editor/document.rs src/ui/root.rs tests/editor_document.rs tests/ui_state.rs
git commit -m "feat: add project file autosave"
```

### Task 18: Protect dirty files when closing files, projects, and the window

**Files:**

- Modify: `src/ui/root.rs`
- Modify: `src/ui/root/dialogs.rs`
- Modify: `src/ui/app.rs`
- Modify: `src/ui/i18n.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_i18n.rs`

- [ ] **Step 1: Write failing file-close decision tests**

Test clean close is immediate; dirty close yields Save/Discard/Cancel; save failure keeps the Tab; cancel restores the same active file; discard removes only that file and selects the approved right-then-left neighbor.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_state closing_dirty_file_requires_a_decision -- --exact`

Expected: FAIL because `TabClose` still assumes a terminal tab.

- [ ] **Step 3: Add file-close pending state/dialog**

Use `SaveContinuation::CloseFile(document_id)` for Save. Discard removes the document entity/subscription/session item without writing. Cancel mutates nothing. Give file-close state higher input-owner/dialog priority than the ordinary project editor.

- [ ] **Step 4: Write failing combined project-close blocker tests**

Cover dirty-only, running-terminal-only, and dirty+running cases. Require one pending close model that includes dirty document IDs and running pane count, instead of serial dialogs. Saving all must close only after every write succeeds.

- [ ] **Step 5: Replace `pending_close_project_id` with an explicit close intent**

```rust
pub enum CloseIntent {
    Project(ProjectId),
    Window,
}

pub struct PendingClose {
    intent: CloseIntent,
    dirty_documents: Vec<DocumentId>,
    running_pane_count: usize,
}
```

Actions are **Save All and Continue**, **Discard and Continue**, and **Cancel**. Preserve existing running-process confirmation/termination semantics. A failed save leaves the project/window open and the pending close recoverable.

- [ ] **Step 6: Add a failing window-close interception test**

Use a GPUI `VisualTestContext` and `simulate_close()`. A dirty document must make the first close request return false and show pending window-close state; a clean/allowed retry must return true.

- [ ] **Step 7: Register GPUI's close guard**

In `ui/app.rs`, after creating the root entity, call `window.on_window_should_close`. The callback updates the weak RootView and returns `root.request_window_close(...)`. Use an `allow_window_close_once` flag for the confirmed retry so programmatic close does not reopen the dialog.

- [ ] **Step 8: Localize all close protection text**

Include file names, dirty-file count, running-process count, Save All, Discard, Cancel, and failed-save guidance in English and Chinese.

- [ ] **Step 9: Run close tests**

Run: `cargo test --test ui_state close -- --nocapture`

Run: `cargo test --test ui_i18n close -- --nocapture`

Expected: old terminal close tests and new file/project/window cases PASS.

- [ ] **Step 10: Commit**

```bash
git add src/ui/root.rs src/ui/root/dialogs.rs src/ui/app.rs src/ui/i18n.rs tests/ui_state.rs tests/ui_i18n.rs
git commit -m "feat: protect unsaved project files on close"
```

### Task 19: Expose editor/project-panel settings in Settings and apply them safely

**Files:**

- Modify: `src/ui/settings.rs`
- Modify: `src/ui/font_options.rs`
- Modify: `src/ui/root.rs:32-45, 180-198, 730-855, 2099-2195, 2310-2525, 2950-3065, 4297-4622`
- Modify: `src/ui/i18n.rs`
- Modify: `tests/ui_shell.rs`
- Modify: `tests/ui_state.rs`
- Modify: `tests/ui_i18n.rs`

- [ ] **Step 1: Write failing Settings metadata tests**

Add `SettingsGroupId::Editor` and require rows for font family, font size, line height, tab width, soft wrap, line numbers, autosave, autosave delay, default-open file tree, hidden files, right width, and left width. Keep Languages focused on detection/LSP.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --test ui_shell editor_settings_rows_expose_effective_controls -- --exact`

Expected: FAIL because the Editor group/rows are missing.

- [ ] **Step 3: Generalize font option helpers**

Introduce generic `font_family_options_from_system`, `font_family_option_for_setting`, and `font_family_setting_from_option`; keep the terminal-named functions as wrappers to avoid unrelated call-site/test churn.

- [ ] **Step 4: Add Settings control entities and persistence handlers**

Add a separate editor font select, autosave select, and number inputs for editor/panel fields. Reuse existing `Input`, `Select`, `Switch`, setting-row, and save-error patterns. Disable or hide autosave delay unless `AfterDelay` is selected if the current components support it without extra state; otherwise keep it visible with clear descriptive text.

- [ ] **Step 5: Write failing live-application tests**

Require font/size/line-height/style snapshots to update for all open documents without replacing their entity IDs or changing text/saved baseline; require soft-wrap/line-number setters to run; require show-hidden to refresh visible trees; and require tab size to remain the old value for existing inputs while new/reopened documents use the saved value.

- [ ] **Step 6: Implement safe runtime propagation**

After saving editor display settings, call each `ProjectEditorDocument::set_appearance(window, cx)`. Never call `InputState::set_value` for appearance changes. Autosave changes affect future events/tasks; switching to Off cancels pending delayed tasks. `show_hidden` increments tree generations and refreshes expanded directories. `default_open` affects only new project sessions. Width controls update persisted defaults and the selected session only when explicitly edited through Settings.

- [ ] **Step 7: Localize settings labels/descriptions**

The tab-size description must state that already-open files need to be reopened with gpui-component 0.5.1. Add all English/Chinese strings and search metadata.

- [ ] **Step 8: Run settings/UI tests**

Run: `cargo test --test settings_config --test ui_shell --test ui_state --test ui_i18n settings -- --nocapture`

Expected: all matching tests PASS.

- [ ] **Step 9: Commit**

```bash
git add src/ui/settings.rs src/ui/font_options.rs src/ui/root.rs src/ui/i18n.rs tests/ui_shell.rs tests/ui_state.rs tests/ui_i18n.rs
git commit -m "feat: add effective project editor settings"
```

### Task 20: Update product documentation and run complete verification

**Files:**

- Modify: `README.md`
- Modify: `docs/usage.md`
- Modify as needed only for discovered regressions: files already listed above

- [ ] **Step 1: Update documentation before claiming completion**

Revise README's “not a file explorer, not an editor” statement. Document:

- unified terminal/file work items;
- right project tree and fixed toggle;
- sidebar drag behavior;
- manual save, autosave modes, conflicts, and close protection;
- the complete TOML settings block and defaults;
- 10 MiB/UTF-8/no-watcher/no-file-operations limits;
- `file.save`, `project_panel.toggle`, and `project_panel.refresh` commands.

- [ ] **Step 2: Format**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect the diff, then rerun `cargo fmt --check`.

- [ ] **Step 3: Run focused high-risk suites**

Run:

```bash
cargo test --test project_file_io
cargo test --test project_tree
cargo test --test editor_document
cargo test --test editor_workspace
cargo test --test settings_config
cargo test --test commands_keybindings
cargo test --test palette
cargo test --test ui_shell
cargo test --test ui_state
```

Expected: all PASS; the two real-PTY tests remain ignored only in the full suite.

- [ ] **Step 4: Run the full suite**

Run: `cargo test`

Expected: all non-ignored tests PASS, with no new warnings beyond the pre-existing `block v0.1.6` future-incompatibility notice.

- [ ] **Step 5: Lint all targets/features**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS. If a dependency-only future-incompatibility notice remains, document it; do not suppress new yttt warnings.

- [ ] **Step 6: Check the final diff and worktree scope**

Run: `git diff --check`

Run: `git status --short`

Expected: only planned project files are modified; the user's untracked `AGENTS.md` remains untouched and unstaged.

- [ ] **Step 7: Build and launch the real macOS app for smoke verification**

Run: `scripts/run-dev-app.sh --fixture none --no-open`

Then, with approval for GUI launch, run the bundle launcher with `YTTT_OPEN_PROJECT` set to a disposable test project containing nested folders, hidden files, same-name files, and a Git repository. Use `@computer-use:computer-use` for visible UI checks.

Expected manual checks:

1. Left and right sidebars drag in the correct direction and restore width.
2. Folder button is fixed after the tab scroll area and visibly active only while the right tree is open.
3. Directories load lazily; hidden-file setting and refresh work.
4. Reopening a file deduplicates and focuses its existing Tab.
5. Terminal/file Tab navigation, dirty dot, tooltip, close hover, and project switching work.
6. Font/size/line-height/soft-wrap/line-number settings update open files without losing text/undo; tab size updates after reopen.
7. Manual save and both autosave modes work.
8. External modification/deletion presents the correct conflict choices.
9. Dirty file, dirty project with running terminals, and window close all protect data.

- [ ] **Step 8: Commit documentation and any verified final corrections**

```bash
git add README.md docs/usage.md
git commit -m "docs: document project file editing"
```

- [ ] **Step 9: Re-run completion gate after the final commit**

Run: `cargo fmt --check`

Run: `cargo test`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: every command exits 0. Record exact pass/ignored counts and any pre-existing dependency warning in the handoff.

### Chunk 3 checkpoint

- [ ] Confirm all manual smoke items above.
- [ ] Confirm no pending save/conflict/autosave task can update a closed project/document.
- [ ] Confirm the implementation matches `docs/superpowers/specs/2026-07-10-project-file-tree-editor-design.md`, including the approved live-tab-size exception.
- [ ] Do not launch a code-review or completion-review subagent automatically; ask the user first, per `AGENTS.md`.

---


## Inline execution order

Execute Tasks 1–20 in order with `@superpowers:executing-plans` and `@superpowers:test-driven-development`. Stop at each chunk checkpoint to report tests and unexpected design/API discoveries. Do not batch production code ahead of its failing test. Do not use subagents for implementation or review unless the user explicitly changes the repository instruction.
