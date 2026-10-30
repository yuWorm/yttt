use super::host_mirror_tests::{open_window, wait_until};
use super::*;
use crate::ui::terminal::pane::recovery_tests::RecoveryHost;

fn edit(document: &Entity<ProjectEditorDocument>, text: &str, cx: &mut gpui::VisualTestContext) {
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, cx| {
        input.set_value("", window, cx);
        input.replace(text, window, cx);
    });
}

fn loaded(root: &WorkbenchView, document_id: &DocumentId) -> Option<Entity<ProjectEditorDocument>> {
    root.project
        .project_editor_runtime
        .document(document_id)
        .cloned()
}

fn held_draft(root: &WorkbenchView, document_id: &DocumentId, cx: &App) -> Option<String> {
    loaded(root, document_id)?
        .read(cx)
        .held_elsewhere()?
        .draft
        .clone()
}

#[gpui::test]
fn one_client_edits_a_document_while_the_other_reads_its_draft_until_taking_over(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    let host = RecoveryHost::start();
    let project = host.root.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("notes.txt"), "on disk").unwrap();
    let first = host.client("first");
    let second = host.client("second");
    let (a, mut a_cx) = open_window(cx, &first, &project, host.root.path().join("a"), None);
    let (workspace_id, project_id) = a.read_with(&a_cx, |root, _| {
        (
            root.workspace_persistence
                .view
                .as_ref()
                .unwrap()
                .id()
                .clone(),
            root.workspace.opened_projects()[0].id.clone(),
        )
    });
    let document_id = DocumentId {
        project_id,
        canonical_path: crate::config::paths::canonicalize_path(&project.join("notes.txt"))
            .unwrap(),
    };
    a.update_in(&mut a_cx, |root, _, _| {
        root.open_file_work_item(&document_id).unwrap();
    });
    let view = second.claim_history_workspace(workspace_id).unwrap();
    let (b, mut b_cx) = open_window(
        cx,
        &second,
        &project,
        host.root.path().join("b"),
        Some(view),
    );
    wait_until(&mut a_cx, &a, "loading the file", |root, cx| {
        loaded(root, &document_id)
            .is_some_and(|document| document.read(cx).model().value() == "on disk")
    });
    wait_until(&mut b_cx, &b, "mirroring the open file", |root, cx| {
        loaded(root, &document_id)
            .is_some_and(|document| document.read(cx).model().value() == "on disk")
    });

    // The first Client's edits reach the second as a read-only draft.
    let a_document = a.read_with(&a_cx, |root, _| loaded(root, &document_id).unwrap());
    edit(&a_document, "uploaded edit", &mut a_cx);
    wait_until(
        &mut b_cx,
        &b,
        "showing the other Client's draft",
        |root, cx| held_draft(root, &document_id, cx).as_deref() == Some("uploaded edit"),
    );
    b.read_with(&b_cx, |root, cx| {
        let document = loaded(root, &document_id).unwrap();
        assert_eq!(document.read(cx).model().value(), "on disk");
        assert!(!document.read(cx).model().is_dirty());
    });

    // Without a tick the first Client has not uploaded this edit when the
    // second takes the document over.
    edit(&a_document, "unuploaded edit", &mut a_cx);
    b.update_in(&mut b_cx, |root, window, cx| {
        root.take_over_document(document_id.clone(), window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let copies = host.root.path().join("a/device-state/document-copies");
    loop {
        b_cx.run_until_parked();
        let b_text = b.read_with(&b_cx, |root, cx| {
            loaded(root, &document_id).map(|document| document.read(cx).model().value().to_string())
        });
        let copied = fs::read_dir(&copies)
            .ok()
            .and_then(|mut entries| entries.next());
        if b_text.as_deref() == Some("uploaded edit") && copied.is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "taking over: {b_text:?}, {:?}",
            a.read_with(&a_cx, |root, _| root
                .workspace_persistence
                .last_error
                .clone())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let copy = fs::read_dir(&copies)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read_to_string(copy).unwrap(), "unuploaded edit");
    b.read_with(&b_cx, |root, cx| {
        let document = loaded(root, &document_id).unwrap();
        assert!(document.read(cx).held_elsewhere().is_none());
        assert!(document.read(cx).model().is_dirty());
    });
    // The first Client returns to the file on disk and shows the new
    // holder's draft read-only.
    wait_until(
        &mut a_cx,
        &a,
        "reading the new holder's draft",
        |root, cx| {
            held_draft(root, &document_id, cx).as_deref() == Some("uploaded edit")
                && loaded(root, &document_id)
                    .is_some_and(|document| document.read(cx).model().value() == "on disk")
        },
    );
}
