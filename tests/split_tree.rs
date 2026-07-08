use yttt::model::split_tree::{FocusDirection, SplitDirection, SplitTree};

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

#[test]
fn focuses_left_and_right_across_horizontal_split() {
    let mut tree = SplitTree::single("shell");
    tree.split_focused(SplitDirection::Horizontal, "server")
        .unwrap();

    tree.focus_direction(FocusDirection::Left).unwrap();
    assert_eq!(tree.focused_pane_id(), Some("shell"));

    tree.focus_direction(FocusDirection::Right).unwrap();
    assert_eq!(tree.focused_pane_id(), Some("server"));
}

#[test]
fn focuses_up_and_down_across_vertical_split() {
    let mut tree = SplitTree::single("top");
    tree.split_focused(SplitDirection::Vertical, "bottom")
        .unwrap();

    tree.focus_direction(FocusDirection::Up).unwrap();
    assert_eq!(tree.focused_pane_id(), Some("top"));

    tree.focus_direction(FocusDirection::Down).unwrap();
    assert_eq!(tree.focused_pane_id(), Some("bottom"));
}
