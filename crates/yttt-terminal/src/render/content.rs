use crate::colors::ColorPalette;
use crate::event::GpuiEventProxy;
use alacritty_terminal::grid::{Dimensions, Indexed};
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui::Hsla;
use smallvec::SmallVec;
use std::num::NonZeroU32;
use std::ops::RangeInclusive;

const DIM_FACTOR: f32 = 0.66;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalCellWidth {
    Single,
    Wide,
    Spacer,
    LeadingSpacer,
}

impl TerminalCellWidth {
    pub(crate) fn columns(self) -> usize {
        match self {
            Self::Wide => 2,
            Self::Single | Self::Spacer | Self::LeadingSpacer => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct TerminalFontStyle {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct RenderDecorationFlags(u8);

impl RenderDecorationFlags {
    pub const UNDERLINE: Self = Self(1 << 0);
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 1);
    pub const UNDERCURL: Self = Self(1 << 2);
    pub const DOTTED_UNDERLINE: Self = Self(1 << 3);
    pub const DASHED_UNDERLINE: Self = Self(1 << 4);
    pub const STRIKEOUT: Self = Self(1 << 5);

    pub(crate) fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    fn from_cell_flags(flags: Flags) -> Self {
        let mut decorations = Self::default();
        for (flag, decoration) in [
            (Flags::UNDERLINE, Self::UNDERLINE),
            (Flags::DOUBLE_UNDERLINE, Self::DOUBLE_UNDERLINE),
            (Flags::UNDERCURL, Self::UNDERCURL),
            (Flags::DOTTED_UNDERLINE, Self::DOTTED_UNDERLINE),
            (Flags::DASHED_UNDERLINE, Self::DASHED_UNDERLINE),
            (Flags::STRIKEOUT, Self::STRIKEOUT),
        ] {
            if flags.contains(flag) {
                decorations.0 |= decoration.0;
            }
        }
        decorations
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RenderableCell {
    pub point: AlacPoint,
    pub text: SmallVec<[char; 3]>,
    pub width: TerminalCellWidth,
    pub foreground: Hsla,
    pub background: Hsla,
    pub underline_color: Hsla,
    pub font_style: TerminalFontStyle,
    pub decorations: RenderDecorationFlags,
    pub selected: bool,
    pub hyperlink: Option<Hyperlink>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RenderableRow {
    pub line: Line,
    pub cells: Vec<RenderableCell>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RenderableCursor {
    pub point: AlacPoint<usize>,
    pub shape: CursorShape,
    pub cursor_color: Hsla,
    pub text_color: Hsla,
    pub width: NonZeroU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineDamageBounds {
    pub line: usize,
    pub left: usize,
    pub right: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RenderDamage {
    Full,
    Partial(Vec<LineDamageBounds>),
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalRenderSnapshot {
    pub rows: Vec<RenderableRow>,
    pub cursor: RenderableCursor,
    pub display_offset: usize,
    pub cols: usize,
    pub screen_lines: usize,
    pub default_background: Hsla,
    pub default_foreground: Hsla,
    pub damage: RenderDamage,
    pub history_size: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct HintCellOverlay {
    pub point: AlacPoint,
    pub label: Option<char>,
    pub is_start: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RenderOverlayState {
    pub hints: Vec<HintCellOverlay>,
    pub search_matches: Vec<RangeInclusive<AlacPoint>>,
    pub focused_search_match: Option<RangeInclusive<AlacPoint>>,
    pub hovered_hyperlink: Option<RangeInclusive<AlacPoint>>,
}

impl RenderOverlayState {
    fn hint_at(&self, point: AlacPoint) -> Option<&HintCellOverlay> {
        self.hints.iter().find(|hint| hint.point == point)
    }

    fn search_at(&self, point: AlacPoint) -> Option<bool> {
        let matched = self
            .search_matches
            .iter()
            .any(|range| range.contains(&point));
        matched.then(|| {
            self.focused_search_match
                .as_ref()
                .is_some_and(|range| range.contains(&point))
        })
    }

    fn hyperlink_hovered(&self, point: AlacPoint) -> bool {
        self.hovered_hyperlink
            .as_ref()
            .is_some_and(|range| range.contains(&point))
    }
}

impl TerminalRenderSnapshot {
    pub(crate) fn build(
        term: &mut Term<GpuiEventProxy>,
        palette: &ColorPalette,
        overlays: &RenderOverlayState,
        focused: bool,
        cursor_unfocused_hollow: bool,
        cursor_visible: bool,
        forced_rows: &[usize],
        generation: u64,
    ) -> Self {
        let cols = term.columns();
        let screen_lines = term.screen_lines();
        let mut damage = match term.damage() {
            alacritty_terminal::term::TermDamage::Full => RenderDamage::Full,
            alacritty_terminal::term::TermDamage::Partial(lines) => RenderDamage::Partial(
                lines
                    .map(|line| LineDamageBounds {
                        line: line.line,
                        left: line.left,
                        right: line.right,
                    })
                    .collect(),
            ),
        };
        if let RenderDamage::Partial(lines) = &mut damage {
            lines.extend(
                forced_rows
                    .iter()
                    .copied()
                    .filter(|line| *line < screen_lines)
                    .map(|line| LineDamageBounds {
                        line,
                        left: 0,
                        right: cols.saturating_sub(1),
                    }),
            );
            lines.sort_unstable_by_key(|line| line.line);
            let mut merged: Vec<LineDamageBounds> = Vec::with_capacity(lines.len());
            for line in lines.drain(..) {
                if let Some(previous) = merged.last_mut()
                    && previous.line == line.line
                {
                    previous.left = previous.left.min(line.left);
                    previous.right = previous.right.max(line.right);
                } else {
                    merged.push(line);
                }
            }
            *lines = merged;
        }
        let mut damaged_rows = vec![false; screen_lines];
        match &damage {
            RenderDamage::Full => damaged_rows.fill(true),
            RenderDamage::Partial(lines) => {
                for line in lines {
                    if line.line < screen_lines {
                        damaged_rows[line.line] = true;
                    }
                }
            }
        }

        let content = term.renderable_content();
        let display_offset = content.display_offset;
        let default_background =
            palette.resolve(Color::Named(NamedColor::Background), content.colors);
        let default_foreground =
            palette.resolve(Color::Named(NamedColor::Foreground), content.colors);
        let history_size = term.grid().total_lines().saturating_sub(screen_lines);
        let cursor_grid_point = content.cursor.point;
        let cursor_viewport = term::point_to_viewport(display_offset, cursor_grid_point);
        let mut cursor_shape = content.cursor.shape;
        if !cursor_visible || cursor_viewport.is_none() {
            cursor_shape = CursorShape::Hidden;
        } else if !focused && cursor_unfocused_hollow && cursor_shape == CursorShape::Block {
            cursor_shape = CursorShape::HollowBlock;
        }

        let mut rows = (0..screen_lines)
            .map(|row| RenderableRow {
                line: Line(row as i32 - display_offset as i32),
                cells: Vec::with_capacity(cols),
                generation,
            })
            .collect::<Vec<_>>();

        for indexed in content.display_iter {
            let viewport_line = indexed.point.line.0 + display_offset as i32;
            if !(0..screen_lines as i32).contains(&viewport_line)
                || !damaged_rows[viewport_line as usize]
            {
                continue;
            }
            let cell = resolve_cell(
                indexed,
                content.selection,
                cursor_grid_point,
                cursor_shape,
                content.mode,
                content.colors,
                palette,
                overlays,
                default_foreground,
                default_background,
            );
            rows[viewport_line as usize].cells.push(cell);
        }
        rows.retain(|row| {
            let viewport_line = row.line.0 + display_offset as i32;
            viewport_line >= 0 && damaged_rows[viewport_line as usize]
        });

        let cursor_point = cursor_viewport.unwrap_or_else(|| AlacPoint::new(0, Column(0)));
        let cursor_cell = rows
            .iter_mut()
            .find(|row| row.line == cursor_grid_point.line)
            .and_then(|row| row.cells.get_mut(cursor_point.column.0));
        let (cursor_color, text_color, cursor_width) = if let Some(cell) = cursor_cell {
            let mut cursor_color =
                palette.resolve(Color::Named(NamedColor::Cursor), content.colors);
            let mut text_color = palette.cursor_text().unwrap_or(cell.background);
            if contrast(cursor_color, cell.background) < 1.5 {
                cursor_color = default_foreground;
                text_color = default_background;
            }
            let width = NonZeroU32::new(cell.width.columns() as u32).unwrap();
            if cursor_shape == CursorShape::Block {
                cell.foreground = text_color;
                cell.background = cursor_color;
            }
            (cursor_color, text_color, width)
        } else {
            (
                palette.resolve(Color::Named(NamedColor::Cursor), content.colors),
                default_background,
                NonZeroU32::new(1).unwrap(),
            )
        };

        term.reset_damage();
        Self {
            rows,
            cursor: RenderableCursor {
                point: cursor_point,
                shape: cursor_shape,
                cursor_color,
                text_color,
                width: cursor_width,
            },
            display_offset,
            cols,
            screen_lines,
            default_background,
            default_foreground,
            damage,
            history_size,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_cell(
    indexed: Indexed<&Cell>,
    selection: Option<alacritty_terminal::selection::SelectionRange>,
    cursor_point: AlacPoint,
    cursor_shape: CursorShape,
    _mode: TermMode,
    colors: &alacritty_terminal::term::color::Colors,
    palette: &ColorPalette,
    overlays: &RenderOverlayState,
    default_foreground: Hsla,
    default_background: Hsla,
) -> RenderableCell {
    let cell = *indexed;
    let flags = cell.flags;
    let mut foreground = resolve_foreground(cell.fg, flags, colors, palette);
    let mut background = palette.resolve(cell.bg, colors);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }

    let selected = selection
        .is_some_and(|selection| selection.contains_cell(&indexed, cursor_point, cursor_shape));
    let mut label = None;
    if let Some(hint) = overlays.hint_at(indexed.point) {
        let (hint_foreground, hint_background) = if hint.is_start {
            palette.hint_start_colors()
        } else {
            palette.hint_end_colors()
        };
        foreground = hint_foreground;
        background = hint_background;
        label = hint.label;
    } else if selected {
        foreground = palette.selection_foreground().unwrap_or(foreground);
        background = palette.selection_background();
        if foreground == background && !flags.contains(Flags::HIDDEN) {
            foreground = default_background;
            background = default_foreground;
        }
    } else if let Some(focused) = overlays.search_at(indexed.point) {
        (foreground, background) = if focused {
            palette.focused_search_colors()
        } else {
            palette.search_colors()
        };
    }

    let width = if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
        TerminalCellWidth::LeadingSpacer
    } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
        TerminalCellWidth::Spacer
    } else if flags.contains(Flags::WIDE_CHAR) {
        TerminalCellWidth::Wide
    } else {
        TerminalCellWidth::Single
    };

    let mut text = SmallVec::new();
    if !flags.contains(Flags::HIDDEN)
        && !matches!(
            width,
            TerminalCellWidth::Spacer | TerminalCellWidth::LeadingSpacer
        )
    {
        let character = label.unwrap_or(cell.c);
        if character != '\0' && character != ' ' {
            text.push(character);
            if label.is_none() {
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth.iter().copied());
                }
            }
        }
    }

    let underline_color = cell.underline_color().map_or(foreground, |color| {
        resolve_foreground(color, flags, colors, palette)
    });

    let mut decorations = RenderDecorationFlags::from_cell_flags(flags);
    if overlays.hyperlink_hovered(indexed.point) {
        decorations.0 |= RenderDecorationFlags::UNDERLINE.0;
    }

    RenderableCell {
        point: indexed.point,
        text,
        width,
        foreground,
        background,
        underline_color,
        font_style: TerminalFontStyle {
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            dim: flags.contains(Flags::DIM),
        },
        decorations,
        selected,
        hyperlink: cell.hyperlink(),
    }
}

fn resolve_foreground(
    color: Color,
    flags: Flags,
    colors: &alacritty_terminal::term::color::Colors,
    palette: &ColorPalette,
) -> Hsla {
    match color {
        Color::Spec(_) if flags.contains(Flags::DIM) => dim_hsla(palette.resolve(color, colors)),
        Color::Named(named) if flags.intersects(Flags::DIM) => {
            palette.resolve(Color::Named(named.to_dim()), colors)
        }
        Color::Indexed(index) if flags.contains(Flags::DIM) && index <= 7 => palette.resolve(
            Color::Named(match index {
                0 => NamedColor::DimBlack,
                1 => NamedColor::DimRed,
                2 => NamedColor::DimGreen,
                3 => NamedColor::DimYellow,
                4 => NamedColor::DimBlue,
                5 => NamedColor::DimMagenta,
                6 => NamedColor::DimCyan,
                _ => NamedColor::DimWhite,
            }),
            colors,
        ),
        Color::Indexed(index) if flags.contains(Flags::DIM) && index <= 15 => {
            palette.resolve(Color::Indexed(index - 8), colors)
        }
        _ => palette.resolve(color, colors),
    }
}

fn dim_hsla(mut color: Hsla) -> Hsla {
    color.l *= DIM_FACTOR;
    color
}

fn contrast(left: Hsla, right: Hsla) -> f64 {
    fn luminance(color: Hsla) -> f64 {
        let color = color.to_rgb();
        let channel = |value: f32| {
            let value = value as f64;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    let left = luminance(left);
    let right = luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}
