use yttt::model::split_tree::{SplitDirection, SplitTree};

#[test]
fn splits_focused_pane_vertically() {
    let mut tree = SplitTree::single("shell");

    tree.split_focused(SplitDirection::Vertical, "server")
        .unwrap();

    assert_eq!(tree.pane_ids(), vec!["shell", "server"]);
    assert_eq!(tree.focused_pane_id(), Some("server"));
}

#[test]
fn closes_focused_pane_and_moves_focus() {
    let mut tree = SplitTree::single("shell");
    tree.split_focused(SplitDirection::Horizontal, "agent")
        .unwrap();

    tree.close_focused().unwrap();

    assert_eq!(tree.pane_ids(), vec!["shell"]);
    assert_eq!(tree.focused_pane_id(), Some("shell"));
}
