use gpui::AppContext as _;
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::SystemTime};

use yttt::{
    model::ids::ProjectId,
    ui::editor::{
        CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, DiskFingerprint, DocumentId,
        EditorAppearance, ProjectEditorDocument, ProjectEditorDocumentEvent, ProjectEditorModel,
        ProjectEditorSaveState,
    },
};

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
