use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, StatefulInteractiveElement as _, Window, actions, div,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::SystemTime,
};

use yttt::{
    model::ids::ProjectId,
    ui::editor::{
        CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, DiskFingerprint, DocumentId,
        EditorAppearance, ProjectEditorDocument, ProjectEditorDocumentEvent, ProjectEditorModel,
        ProjectEditorSaveState, VimMode, init_vim_mode,
    },
};
actions!(vim_test, [TestSave]);

#[test]
fn completed_older_save_keeps_newer_edit_dirty() {
    let mut model = project_model("old", fingerprint(3, 1));
    model.on_input_changed("first edit");
    let request = model.begin_save();
    model.on_input_changed("newer edit");

    assert!(model.finish_save(&request, fingerprint(10, 2)));

    assert!(model.is_dirty());
    assert_eq!(model.value(), "newer edit");
    assert_eq!(model.saved_value(), request.text);
    assert_eq!(model.disk_fingerprint(), &fingerprint(10, 2));
    assert_eq!(model.save_state(), &ProjectEditorSaveState::Idle);
}

#[test]
fn replacing_from_disk_updates_value_baseline_generation_and_fingerprint() {
    let mut model = project_model("old", fingerprint(3, 1));
    model.on_input_changed("dirty");
    let previous_generation = model.generation();

    model.replace_from_disk("disk value", fingerprint(10, 3));

    assert_eq!(model.value(), "disk value");
    assert_eq!(model.saved_value(), "disk value");
    assert!(!model.is_dirty());
    assert!(model.generation() > previous_generation);
    assert_eq!(model.disk_fingerprint(), &fingerprint(10, 3));
}

#[test]
fn failed_current_save_returns_to_idle_and_keeps_document_dirty() {
    let mut model = project_model("old", fingerprint(3, 1));
    model.on_input_changed("dirty");
    let request = model.begin_save();
    assert_eq!(
        model.save_state(),
        &ProjectEditorSaveState::Saving {
            generation: request.generation,
        }
    );

    assert!(model.fail_save(&request, "disk full"));

    assert_eq!(model.save_state(), &ProjectEditorSaveState::Idle);
    assert!(model.is_dirty());
    assert_eq!(model.editor().error(), Some("disk full"));
}

#[test]
fn canceling_a_conflicted_save_returns_to_idle_without_changing_text() {
    let mut model = project_model("old", fingerprint(3, 1));
    model.on_input_changed("memory text");
    let request = model.begin_save();

    assert!(model.cancel_save(&request));

    assert_eq!(model.save_state(), &ProjectEditorSaveState::Idle);
    assert_eq!(model.value(), "memory text");
    assert!(model.is_dirty());
    assert_eq!(model.editor().error(), None);
}

#[gpui::test]
fn project_editor_document_syncs_input_changes_and_emits_changed(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let model = project_model("old", fingerprint(3, 1));
    let appearance = EditorAppearance {
        font_family: String::new(),
        font_size: 14.0,
        line_height: 1.4,
        soft_wrap: false,
        line_numbers: true,
    };
    let (document, cx) = cx.add_window_view(|window, entity_cx| {
        ProjectEditorDocument::new(model, appearance, window, entity_cx)
    });
    let input = cx.read(|app| document.read(app).input().clone());
    let input_id = input.entity_id();
    let events = Rc::new(RefCell::new(Vec::new()));
    let subscription = document.update(cx, |_, entity_cx| {
        let events = events.clone();
        entity_cx.subscribe(
            &document,
            move |_, _, event: &ProjectEditorDocumentEvent, _| {
                events.borrow_mut().push(event.clone());
            },
        )
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("", window, input_cx);
        input.replace("changed", window, input_cx);
    });
    document.update_in(cx, |document, window, entity_cx| {
        document.set_appearance(
            EditorAppearance {
                font_family: "JetBrains Mono".to_string(),
                font_size: 16.0,
                line_height: 1.6,
                soft_wrap: true,
                line_numbers: false,
            },
            window,
            entity_cx,
        );
    });
    cx.run_until_parked();

    cx.read(|app| {
        let document = document.read(app);
        assert_eq!(document.model().value(), "changed");
        assert!(document.model().is_dirty());
        assert_eq!(document.input().entity_id(), input_id);
        assert_eq!(document.appearance().font_size, 16.0);
        assert!(document.appearance().soft_wrap);
    });
    assert!(matches!(
        events.borrow().as_slice(),
        [ProjectEditorDocumentEvent::Changed { generation: 1 }]
    ));
    drop(subscription);
}

#[gpui::test]
fn project_editor_document_tracks_breadcrumbs_at_the_cursor(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let model = project_model("mod app {\n    fn render() {}\n}\n", fingerprint(3, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_breadcrumb_header("src/app.rs")
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());

    input.update_in(cx, |input, window, input_cx| {
        input.set_cursor_position(gpui_component::input::Position::new(1, 4), window, input_cx);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|app| document.read(app).breadcrumb_header().to_string()),
        "src/app.rs"
    );
    assert_eq!(
        cx.read(|app| document
            .read(app)
            .model()
            .editor()
            .config()
            .title()
            .to_string()),
        "main.rs"
    );

    let breadcrumb_names = cx.read(|app| {
        document
            .read(app)
            .breadcrumbs()
            .iter()
            .map(|symbol| symbol.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        breadcrumb_names,
        vec!["app".to_string(), "render".to_string()]
    );
}
#[gpui::test]
fn vim_mode_blocks_normal_input_and_handles_unicode_edits(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let model = project_model("a界b", fingerprint(5, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("vim-mode-indicator").is_some());

    cx.read(|app| {
        assert_eq!(document.read(app).vim_mode(), Some(VimMode::Normal));
        assert!(!input.read(app).text_input_enabled());
        assert_eq!(
            input.read(app).cursor_shape(),
            gpui_component::input::InputCursorShape::Block
        );
    });

    cx.simulate_keystrokes("l x");
    assert_eq!(cx.read(|app| input.read(app).value().to_string()), "ab");

    cx.simulate_keystrokes("i");
    cx.read(|app| {
        assert_eq!(document.read(app).vim_mode(), Some(VimMode::Insert));
        assert!(input.read(app).text_input_enabled());
        assert_eq!(
            input.read(app).cursor_shape(),
            gpui_component::input::InputCursorShape::Bar
        );
    });

    cx.simulate_keystrokes("界 escape");
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(input.read(app).value(), "a界b");
        assert_eq!(document.read(app).vim_mode(), Some(VimMode::Normal));
        assert!(!input.read(app).text_input_enabled());
    });
    cx.simulate_keystrokes("r shift-z");
    assert_eq!(cx.read(|app| input.read(app).value()), "aZb");
    cx.simulate_keystrokes("u");
    assert_eq!(cx.read(|app| input.read(app).value()), "a界b");
}

#[gpui::test]
fn vim_operator_count_undo_redo_and_register_paste_are_composable(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let model = project_model("one two three four", fingerprint(18, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("2 w d");
    assert_eq!(
        cx.read(|app| document.read(app).vim_status()),
        Some("NORMAL d".to_string())
    );
    cx.simulate_keystrokes("w");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "one two four"
    );

    cx.simulate_keystrokes("u");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "one two three four"
    );
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "one two four"
    );

    cx.simulate_keystrokes("0 y w shift-g p");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "one two fourone "
    );
}

#[gpui::test]
fn vim_visual_and_linewise_operators_preserve_register_shape(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let model = project_model("one two\nthree\nfour\n", fingerprint(19, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("v e d");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        " two\nthree\nfour\n"
    );
    cx.simulate_keystrokes("u");

    cx.simulate_keystrokes("shift-v j d");
    assert_eq!(cx.read(|app| input.read(app).value().to_string()), "four\n");
    cx.simulate_keystrokes("u");

    cx.simulate_keystrokes("2 d d p");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "four\none two\nthree\n"
    );
}

#[gpui::test]
fn vim_distinguishes_logical_and_soft_wrapped_display_lines(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let source = format!("{}\nshort", "x".repeat(300));
    let model = project_model(&source, fingerprint(source.len() as u64, 1));
    let mut appearance = EditorAppearance::default();
    appearance.soft_wrap = true;
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) =
        cx.add_window_view(move |window, entity_cx| {
            let document =
                entity_cx.new(|document_cx| {
                    ProjectEditorDocument::new(model, appearance, window, document_cx)
                        .with_vim_mode(true, window, document_cx)
                });
            *document_slot_for_window.borrow_mut() = Some(document.clone());
            gpui_component::Root::new(document, window, entity_cx)
        });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("g j");
    let display_line_position = cx.read(|app| input.read(app).cursor_position());
    assert_eq!(display_line_position.line, 0);
    assert!(display_line_position.character > 0);

    input.update_in(cx, |input, window, input_cx| {
        input.set_cursor_position(gpui_component::input::Position::new(0, 0), window, input_cx);
    });
    cx.simulate_keystrokes("j");
    assert_eq!(cx.read(|app| input.read(app).cursor_position().line), 1);
}

#[gpui::test]
fn vim_modes_are_per_document_and_registers_are_shared(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let first_slot = Rc::new(RefCell::new(None));
    let second_slot = Rc::new(RefCell::new(None));
    let first_slot_for_window = first_slot.clone();
    let second_slot_for_window = second_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let first = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("one two", fingerprint(7, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        let second = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("X", fingerprint(1, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *first_slot_for_window.borrow_mut() = Some(first.clone());
        *second_slot_for_window.borrow_mut() = Some(second.clone());
        let pair = entity_cx.new(|_| VimDocumentPair { first, second });
        gpui_component::Root::new(pair, window, entity_cx)
    });
    let first = first_slot.borrow_mut().take().unwrap();
    let second = second_slot.borrow_mut().take().unwrap();

    first.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("y w i");
    assert_eq!(
        cx.read(|app| first.read(app).vim_mode()),
        Some(VimMode::Insert)
    );

    second.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|app| second.read(app).vim_mode()),
        Some(VimMode::Normal)
    );
    cx.simulate_keystrokes("p");
    assert_eq!(
        cx.read(|app| second.read(app).input().read(app).value().to_string()),
        "Xone "
    );
}
struct VimDocumentPair {
    first: Entity<ProjectEditorDocument>,
    second: Entity<ProjectEditorDocument>,
}

impl Render for VimDocumentPair {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.first.clone()).child(self.second.clone())
    }
}
struct VimShortcutHost {
    document: Entity<ProjectEditorDocument>,
    save_count: Rc<Cell<usize>>,
}

impl VimShortcutHost {
    fn on_save(&mut self, _: &TestSave, _: &mut Window, _: &mut Context<Self>) {
        self.save_count.set(self.save_count.get() + 1);
    }
}

impl Render for VimShortcutHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("VimTestWorkspace")
            .on_action(cx.listener(Self::on_save))
            .child(self.document.clone())
    }
}

#[gpui::test]
fn vim_change_is_one_undo_transaction(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let model = project_model("alpha beta", fingerprint(10, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("c w shift-x escape");
    assert_eq!(
        cx.read(|app| document.read(app).input().read(app).value().to_string()),
        "X beta"
    );
    cx.simulate_keystrokes("u");
    assert_eq!(
        cx.read(|app| document.read(app).input().read(app).value().to_string()),
        "alpha beta"
    );
}

#[gpui::test]
fn vim_bindings_do_not_capture_search_panel_input(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let model = project_model("one two", fingerprint(7, 1));
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
                .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("/");
    cx.run_until_parked();
    cx.simulate_keystrokes("d w");
    cx.read(|app| {
        assert_eq!(document.read(app).vim_mode(), Some(VimMode::Normal));
        assert_eq!(document.read(app).vim_status(), Some("NORMAL".to_string()));
        assert_eq!(document.read(app).input().read(app).value(), "one two");
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(
        cx.read(|app| document.read(app).vim_mode()),
        Some(VimMode::Normal)
    );
}

#[gpui::test]
fn vim_bindings_are_inert_outside_editor_context(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let input_slot = Rc::new(RefCell::new(None));
    let input_slot_for_window = input_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let input =
            entity_cx.new(|input_cx| gpui_component::input::InputState::new(window, input_cx));
        *input_slot_for_window.borrow_mut() = Some(input.clone());
        gpui_component::Root::new(input, window, entity_cx)
    });
    let input = input_slot.borrow_mut().take().unwrap();
    input.update_in(cx, |input, window, input_cx| {
        input.focus(window, input_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("h i");
    assert_eq!(cx.read(|app| input.read(app).value()), "hi");
}

#[gpui::test]
fn vim_normal_mode_allows_workspace_shortcuts(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
        cx.bind_keys([KeyBinding::new("cmd-s", TestSave, Some("VimTestWorkspace"))]);
    });
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let save_count = Rc::new(Cell::new(0));
    let save_count_for_window = save_count.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("unchanged", fingerprint(9, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        let host = entity_cx.new(|_| VimShortcutHost {
            document,
            save_count: save_count_for_window,
        });
        gpui_component::Root::new(host, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("cmd-s");
    assert_eq!(save_count.get(), 1);
    cx.read(|app| {
        assert_eq!(document.read(app).vim_mode(), Some(VimMode::Normal));
        assert_eq!(document.read(app).input().read(app).value(), "unchanged");
    });
}

#[gpui::test]
fn vim_search_reuses_the_query_for_next_and_previous_matches(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("one x one y one", fingerprint(15, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("/");
    cx.run_until_parked();
    cx.simulate_keystrokes("o n e");
    cx.run_until_parked();
    cx.simulate_keystrokes("escape n");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 6);

    cx.simulate_keystrokes("n");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 12);
    cx.simulate_keystrokes("shift-n");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 6);

    cx.simulate_keystrokes("3 n");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 6);
    assert_eq!(
        cx.read(|app| document.read(app).vim_status()),
        Some("NORMAL".to_string())
    );
    cx.simulate_keystrokes("x");
    assert_eq!(
        cx.read(|app| input.read(app).value().to_string()),
        "one x ne y one"
    );
}

#[gpui::test]
fn vim_reviewed_motion_counts_and_graphemes_stay_bounded(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("e\u{301}x", fingerprint(4, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("x");
    assert_eq!(cx.read(|app| input.read(app).value()), "x");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "abcd\nx\nz", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("l l l j x");
    assert_eq!(cx.read(|app| input.read(app).value()), "abcd\n\nz");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "\nnext", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("x");
    assert_eq!(cx.read(|app| input.read(app).value()), "\nnext");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(
            0..input.text().len(),
            "one two three\nabcd\nx\nlast",
            window,
            input_cx,
        );
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("2 e");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 6);

    input.update_in(cx, |input, _window, input_cx| {
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("2 $");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 17);

    input.update_in(cx, |input, _window, input_cx| {
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("3 shift-g");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 19);

    cx.simulate_keystrokes("3 g g");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 19);

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "one two", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("9 9 9 9 9 9 9 9 w");
    assert_eq!(cx.read(|app| input.read(app).cursor()), 6);
}

#[gpui::test]
fn vim_reviewed_operator_edges_preserve_lines_and_registers(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("one\ntwo\nthree", fingerprint(13, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("j d shift-g");
    assert_eq!(cx.read(|app| input.read(app).value()), "one\n");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "one\ntwo\nthree", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("j c c shift-x escape");
    assert_eq!(cx.read(|app| input.read(app).value()), "one\nX\nthree");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "one\ntwo", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("shift-g y y g g shift-p");
    assert_eq!(cx.read(|app| input.read(app).value()), "two\none\ntwo");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "abc def", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("y w d 0 shift-g p");
    assert_eq!(cx.read(|app| input.read(app).value()), "abc defabc ");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "abc def", window, input_cx);
        input.set_cursor_offset(4, input_cx);
    });
    cx.simulate_keystrokes("2 d 0");
    assert_eq!(cx.read(|app| input.read(app).value()), "def");

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "ab\ncd", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("l 2 r shift-x");
    assert_eq!(cx.read(|app| input.read(app).value()), "aX\ncd");
}

#[gpui::test]
fn vim_normal_mode_native_undo_cannot_consume_history(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_vim_mode(cx);
    });
    let document_slot = Rc::new(RefCell::new(None));
    let document_slot_for_window = document_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, entity_cx| {
        let document = entity_cx.new(|document_cx| {
            ProjectEditorDocument::new(
                project_model("abc", fingerprint(3, 1)),
                EditorAppearance::default(),
                window,
                document_cx,
            )
            .with_vim_mode(true, window, document_cx)
        });
        *document_slot_for_window.borrow_mut() = Some(document.clone());
        gpui_component::Root::new(document, window, entity_cx)
    });
    let document = document_slot.borrow_mut().take().unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("i shift-x escape cmd-z");
    assert_eq!(cx.read(|app| input.read(app).value()), "Xabc");
    cx.simulate_keystrokes("u");
    cx.read(|app| {
        assert_eq!(input.read(app).value(), "abc");
        assert_eq!(input.read(app).cursor(), 0);
    });
    cx.simulate_keystrokes("ctrl-r");
    cx.read(|app| {
        assert_eq!(input.read(app).value(), "Xabc");
        assert_eq!(input.read(app).cursor(), 0);
    });

    input.update_in(cx, |input, window, input_cx| {
        input.replace_range(0..input.text().len(), "abc", window, input_cx);
        input.set_cursor_offset(0, input_cx);
    });
    cx.simulate_keystrokes("l backspace");
    cx.read(|app| {
        assert_eq!(input.read(app).value(), "abc");
        assert_eq!(input.read(app).cursor(), 1);
    });
}

#[test]
fn relocating_model_rekeys_future_saves_without_losing_dirty_text() {
    let mut model = project_model("old", fingerprint(3, 1));
    model.on_input_changed("dirty");
    let _stale_request = model.begin_save();
    let new_document_id = DocumentId {
        project_id: ProjectId::new("project-a"),
        canonical_path: PathBuf::from("/project-a/src/renamed.py"),
    };

    model.relocate(new_document_id.clone(), "renamed.py");

    assert_eq!(model.document_id(), &new_document_id);
    assert_eq!(model.editor().path(), new_document_id.canonical_path);
    assert_eq!(model.editor().config().title(), "renamed.py");
    assert_eq!(model.value(), "dirty");
    assert!(model.is_dirty());
    assert_eq!(model.save_state(), &ProjectEditorSaveState::Idle);
    assert_eq!(model.begin_save().document_id, new_document_id);
}

fn project_model(value: &str, disk_fingerprint: DiskFingerprint) -> ProjectEditorModel {
    let document_id = DocumentId {
        project_id: ProjectId::new("project-a"),
        canonical_path: PathBuf::from("/project-a/src/main.rs"),
    };
    let editor = CodeEditorState::new(
        &document_id.canonical_path,
        CodeEditorConfig::new("main.rs", CodeEditorLanguageMode::Auto),
        value,
    );
    ProjectEditorModel::new(document_id, editor, disk_fingerprint)
}

fn fingerprint(byte_len: u64, content_hash: u64) -> DiskFingerprint {
    DiskFingerprint {
        exists: true,
        byte_len,
        modified: Some(SystemTime::UNIX_EPOCH),
        content_hash,
    }
}
