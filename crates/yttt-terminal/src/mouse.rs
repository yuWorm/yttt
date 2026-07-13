//! Framework-neutral terminal mouse protocol encoding and pointer geometry.

use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::term::TermMode;
use gpui::{Pixels, Point};
use smallvec::{SmallVec, smallvec};

use crate::input::TerminalModifiers;

/// Type of text selection derived from a click count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionType {
    Simple,
    Word,
    Line,
}

/// Mouse buttons representable by the terminal protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalMouseButton {
    Left,
    Middle,
    Right,
}

/// State of a terminal mouse button event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonState {
    Pressed,
    Released,
}

/// Direction of a terminal mouse wheel report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalWheelDirection {
    Up,
    Down,
    Left,
    Right,
}

/// A framework-independent terminal mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMouseEvent {
    Button {
        button: TerminalMouseButton,
        state: MouseButtonState,
        point: AlacPoint,
    },
    Motion {
        held_button: Option<TerminalMouseButton>,
        point: AlacPoint,
    },
    Wheel {
        direction: TerminalWheelDirection,
        point: AlacPoint,
    },
}

/// Convert a pixel position to an unclamped terminal grid point.
pub fn pixel_to_cell(
    position: Point<Pixels>,
    origin: Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
) -> AlacPoint {
    let column = ((position.x - origin.x) / cell_width).floor().max(0.0) as usize;
    let line = ((position.y - origin.y) / cell_height).floor().max(0.0) as i32;
    AlacPoint::new(Line(line), Column(column))
}

/// Return which half of a terminal cell contains a pixel position.
pub fn pixel_to_cell_side(
    position_x: Pixels,
    origin_x: Pixels,
    cell_width: Pixels,
    columns: usize,
) -> Side {
    let relative: f32 = (position_x - origin_x).into();
    let width: f32 = cell_width.into();
    if relative <= 0.0 || width <= 0.0 {
        return Side::Left;
    }

    let grid_width = width * columns as f32;
    let within_cell = relative % width;
    if relative >= grid_width || within_cell > width / 2.0 {
        Side::Right
    } else {
        Side::Left
    }
}

/// Determine selection granularity from the click count.
pub fn selection_type_from_clicks(click_count: usize) -> SelectionType {
    match click_count {
        1 => SelectionType::Simple,
        2 => SelectionType::Word,
        _ => SelectionType::Line,
    }
}

/// Encode a terminal mouse event according to the active protocol modes.
pub fn encode_mouse(
    event: TerminalMouseEvent,
    modifiers: TerminalModifiers,
    mode: TermMode,
) -> Option<SmallVec<[u8; 32]>> {
    let mouse_mode = mode
        .intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION);
    if !mouse_mode {
        return None;
    }

    let (point, mut code, state) = match event {
        TerminalMouseEvent::Button {
            button,
            state,
            point,
        } => (point, button_code(button), state),
        TerminalMouseEvent::Wheel { direction, point } => {
            (point, wheel_code(direction), MouseButtonState::Pressed)
        }
        TerminalMouseEvent::Motion { held_button, point } => {
            if !mode.contains(TermMode::MOUSE_MOTION)
                && !(mode.contains(TermMode::MOUSE_DRAG) && held_button.is_some())
            {
                return None;
            }
            let button = held_button.map_or(3, button_code);
            (point, button | 32, MouseButtonState::Pressed)
        }
    };

    if point.line.0 < 0 {
        return None;
    }

    code |= modifier_code(modifiers);
    if mode.contains(TermMode::SGR_MOUSE) {
        Some(encode_sgr(point, code, state))
    } else {
        if state == MouseButtonState::Released {
            code = 3 | modifier_code(modifiers);
        }
        encode_normal(point, code, mode.contains(TermMode::UTF8_MOUSE))
    }
}

fn button_code(button: TerminalMouseButton) -> u8 {
    match button {
        TerminalMouseButton::Left => 0,
        TerminalMouseButton::Middle => 1,
        TerminalMouseButton::Right => 2,
    }
}

fn wheel_code(direction: TerminalWheelDirection) -> u8 {
    match direction {
        TerminalWheelDirection::Up => 64,
        TerminalWheelDirection::Down => 65,
        TerminalWheelDirection::Left => 66,
        TerminalWheelDirection::Right => 67,
    }
}

fn modifier_code(modifiers: TerminalModifiers) -> u8 {
    (u8::from(modifiers.shift) << 2)
        | (u8::from(modifiers.alt) << 3)
        | (u8::from(modifiers.control) << 4)
}

fn encode_sgr(point: AlacPoint, code: u8, state: MouseButtonState) -> SmallVec<[u8; 32]> {
    let mut bytes = smallvec![b'\x1b', b'[', b'<'];
    push_decimal(&mut bytes, code as usize);
    bytes.push(b';');
    push_decimal(&mut bytes, point.column.0 + 1);
    bytes.push(b';');
    push_decimal(&mut bytes, point.line.0 as usize + 1);
    bytes.push(match state {
        MouseButtonState::Pressed => b'M',
        MouseButtonState::Released => b'm',
    });
    bytes
}

fn encode_normal(point: AlacPoint, code: u8, utf8: bool) -> Option<SmallVec<[u8; 32]>> {
    let max_point = if utf8 { 2015 } else { 223 };
    if point.column.0 >= max_point || point.line.0 as usize >= max_point {
        return None;
    }

    let mut bytes = smallvec![b'\x1b', b'[', b'M', 32 + code];
    encode_normal_position(&mut bytes, point.column.0, utf8);
    encode_normal_position(&mut bytes, point.line.0 as usize, utf8);
    Some(bytes)
}

fn encode_normal_position(bytes: &mut SmallVec<[u8; 32]>, position: usize, utf8: bool) {
    if utf8 && position >= 95 {
        let position = 33 + position;
        bytes.push((0xc0 + position / 64) as u8);
        bytes.push((0x80 + (position & 63)) as u8);
    } else {
        bytes.push(33 + position as u8);
    }
}

fn push_decimal(bytes: &mut SmallVec<[u8; 32]>, mut value: usize) {
    let mut digits = [0_u8; 20];
    let mut cursor = digits.len();
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    bytes.extend_from_slice(&digits[cursor..]);
}

/// Calculate the number of terminal lines represented by a pixel delta.
pub fn pixels_to_scroll_lines(pixel_delta: Pixels, cell_height: Pixels) -> i32 {
    let lines = (pixel_delta / cell_height).round();
    lines.clamp(-10.0, 10.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn cell(line: i32, column: usize) -> AlacPoint {
        AlacPoint::new(Line(line), Column(column))
    }

    fn bytes(
        event: TerminalMouseEvent,
        modifiers: TerminalModifiers,
        mode: TermMode,
    ) -> Option<Vec<u8>> {
        encode_mouse(event, modifiers, mode).map(|bytes| bytes.to_vec())
    }

    #[test]
    fn pixel_coordinates_and_cell_sides_are_stable() {
        let origin = gpui::point(px(10.0), px(10.0));
        assert_eq!(
            pixel_to_cell(gpui::point(px(105.0), px(55.0)), origin, px(10.0), px(20.0),),
            cell(2, 9)
        );
        assert_eq!(
            pixel_to_cell_side(px(14.0), px(10.0), px(10.0), 8),
            Side::Left
        );
        assert_eq!(
            pixel_to_cell_side(px(16.0), px(10.0), px(10.0), 8),
            Side::Right
        );
        assert_eq!(
            pixel_to_cell_side(px(100.0), px(10.0), px(10.0), 8),
            Side::Right
        );
    }

    #[test]
    fn selection_type_follows_click_count() {
        assert_eq!(selection_type_from_clicks(1), SelectionType::Simple);
        assert_eq!(selection_type_from_clicks(2), SelectionType::Word);
        assert_eq!(selection_type_from_clicks(3), SelectionType::Line);
        assert_eq!(selection_type_from_clicks(9), SelectionType::Line);
    }

    #[test]
    fn mouse_protocol_vectors_match_alacritty() {
        let left_press = TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            state: MouseButtonState::Pressed,
            point: cell(5, 10),
        };
        assert_eq!(
            bytes(left_press, TerminalModifiers::default(), TermMode::empty()),
            None
        );
        assert_eq!(
            bytes(
                left_press,
                TerminalModifiers {
                    shift: true,
                    alt: true,
                    control: true,
                    super_key: false,
                },
                TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
            ),
            Some(b"\x1b[<28;11;6M".to_vec())
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Right,
                    state: MouseButtonState::Released,
                    point: cell(5, 10),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
            ),
            Some(b"\x1b[<2;11;6m".to_vec())
        );

        let motion = TerminalMouseEvent::Motion {
            held_button: Some(TerminalMouseButton::Left),
            point: cell(0, 0),
        };
        assert_eq!(
            bytes(
                motion,
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK
            ),
            None
        );
        assert_eq!(
            bytes(
                motion,
                TerminalModifiers::default(),
                TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE,
            ),
            Some(b"\x1b[<32;1;1M".to_vec())
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Motion {
                    held_button: None,
                    point: cell(0, 0),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE,
            ),
            None
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Motion {
                    held_button: None,
                    point: cell(0, 0),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE,
            ),
            Some(b"\x1b[<35;1;1M".to_vec())
        );

        for (direction, code) in [
            (TerminalWheelDirection::Up, 64),
            (TerminalWheelDirection::Down, 65),
            (TerminalWheelDirection::Left, 66),
            (TerminalWheelDirection::Right, 67),
        ] {
            let expected = format!("\x1b[<{code};3;2M").into_bytes();
            assert_eq!(
                bytes(
                    TerminalMouseEvent::Wheel {
                        direction,
                        point: cell(1, 2),
                    },
                    TerminalModifiers::default(),
                    TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
                ),
                Some(expected)
            );
        }

        assert_eq!(
            bytes(
                left_press,
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK,
            ),
            Some(vec![0x1b, b'[', b'M', 32, 43, 38])
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Right,
                    state: MouseButtonState::Released,
                    point: cell(0, 0),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK,
            ),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Left,
                    state: MouseButtonState::Pressed,
                    point: cell(95, 95),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK | TermMode::UTF8_MOUSE,
            ),
            Some(vec![0x1b, b'[', b'M', 32, 0xc2, 0x80, 0xc2, 0x80])
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Left,
                    state: MouseButtonState::Pressed,
                    point: cell(0, 223),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK,
            ),
            None
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Left,
                    state: MouseButtonState::Pressed,
                    point: cell(2015, 0),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK | TermMode::UTF8_MOUSE,
            ),
            None
        );
        assert_eq!(
            bytes(
                TerminalMouseEvent::Wheel {
                    direction: TerminalWheelDirection::Up,
                    point: cell(-1, 0),
                },
                TerminalModifiers::default(),
                TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
            ),
            None
        );
    }

    #[test]
    fn pixels_to_scroll_lines_rounds_and_clamps() {
        assert_eq!(pixels_to_scroll_lines(px(60.0), px(20.0)), 3);
        assert_eq!(pixels_to_scroll_lines(px(-40.0), px(20.0)), -2);
        assert_eq!(pixels_to_scroll_lines(px(500.0), px(20.0)), 10);
    }
}
