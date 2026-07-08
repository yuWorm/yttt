use yttt::model::split_tree::{FocusDirection, ResizeDirection, SplitDirection, SplitTree};

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

#[test]
fn resizes_focused_pane_across_horizontal_split() {
    let mut tree = SplitTree::single("left");
    tree.split_focused(SplitDirection::Horizontal, "right")
        .unwrap();
    tree.focus_direction(FocusDirection::Left).unwrap();

    tree.resize_focused(ResizeDirection::Right, 0.1).unwrap();
    assert_ratio(tree.root_ratio(), 0.6);

    tree.resize_focused(ResizeDirection::Left, 0.2).unwrap();
    assert_ratio(tree.root_ratio(), 0.4);
}

#[test]
fn clamps_split_resize_ratio() {
    let mut tree = SplitTree::single("left");
    tree.split_focused(SplitDirection::Horizontal, "right")
        .unwrap();
    tree.focus_direction(FocusDirection::Left).unwrap();

    tree.resize_focused(ResizeDirection::Right, 2.0).unwrap();
    assert_ratio(tree.root_ratio(), 0.9);

    tree.resize_focused(ResizeDirection::Left, 2.0).unwrap();
    assert_ratio(tree.root_ratio(), 0.1);
}

fn assert_ratio(actual: Option<f32>, expected: f32) {
    let actual = actual.unwrap();
    assert!(
        (actual - expected).abs() < 0.001,
        "expected ratio {expected}, got {actual}"
    );
}
