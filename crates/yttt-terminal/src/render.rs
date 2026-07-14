//! GPUI terminal snapshot rendering.
//!
//! Terminal semantics are resolved under the Alacritty `Term` lock by
//! [`TerminalRenderSnapshot`]. GPUI shaping and painting consume only that
//! framework-neutral snapshot, so no grid lock is held while fonts are shaped.

mod cache;
mod content;

pub(crate) use cache::TerminalRenderCache;
pub(crate) use content::TerminalCellWidth;
pub(crate) use content::{
    HintCellOverlay, RenderDecorationFlags, RenderOverlayState, RenderableCell, RenderableCursor,
    RenderableRow, TerminalFontStyle, TerminalRenderSnapshot,
};

use crate::colors::ColorPalette;
use crate::terminal::TerminalScrollbarMetrics;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui::{
    App, Bounds, Edges, Font, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Point, ShapedLine,
    SharedString, Size, TextAlign, TextRun, UnderlineStyle, Window, px, quad, transparent_black,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq)]
struct BackgroundSpan {
    row: usize,
    start_col: usize,
    end_col: usize,
    color: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DecorationSpan {
    row: usize,
    start_col: usize,
    end_col: usize,
    color: Hsla,
    kind: RenderDecorationFlags,
}

#[derive(Clone, Debug, PartialEq)]
struct TerminalTextBatch {
    column: usize,
    cell_count: usize,
    column_count: usize,
    cell_columns: usize,
    cell_starts: Option<Vec<(usize, usize)>>,
    text: String,
    font_style: TerminalFontStyle,
    foreground: Hsla,
}

impl TerminalTextBatch {
    fn new(cell: &RenderableCell) -> Self {
        Self {
            column: cell.point.column.0,
            cell_count: 1,
            column_count: cell.width.columns(),
            cell_columns: cell.width.columns(),
            cell_starts: (cell.width.columns() > 1).then(|| vec![(0, 0)]),
            text: cell.text.iter().collect(),
            font_style: cell.font_style,
            foreground: cell.foreground,
        }
    }

    fn can_append(&self, cell: &RenderableCell) -> bool {
        self.column + self.column_count == cell.point.column.0
            && self.cell_columns == cell.width.columns()
            && self.font_style == cell.font_style
            && self.foreground == cell.foreground
    }

    fn contains_spacer(&self, cell: &RenderableCell) -> bool {
        cell.width == TerminalCellWidth::Spacer
            && self.column + self.column_count == cell.point.column.0 + 1
    }

    fn append(&mut self, cell: &RenderableCell) {
        if let Some(cell_starts) = self.cell_starts.as_mut() {
            cell_starts.push((self.text.len(), self.column_count));
        }
        self.text.extend(cell.text.iter());
        self.cell_count += 1;
        self.column_count += self.cell_columns;
    }
}

fn text_batches(cells: &[RenderableCell]) -> Vec<TerminalTextBatch> {
    let mut batches = Vec::with_capacity(cells.len() / 8 + 1);
    let mut current = None::<TerminalTextBatch>;

    for cell in cells {
        if cell.text.is_empty() {
            if current
                .as_ref()
                .is_some_and(|batch| batch.contains_spacer(cell))
            {
                continue;
            }
            if let Some(batch) = current.take() {
                batches.push(batch);
            }
            continue;
        }

        if let Some(batch) = current.as_mut()
            && batch.can_append(cell)
        {
            batch.append(cell);
            continue;
        }

        if let Some(batch) = current.replace(TerminalTextBatch::new(cell)) {
            batches.push(batch);
        }
    }

    if let Some(batch) = current {
        batches.push(batch);
    }
    batches
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerminalFontMetrics {
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub ascent: Pixels,
    pub descent: Pixels,
    pub scale_factor: f32,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct FontMetricsKey {
    family: String,
    font_size_bits: u32,
    line_height_bits: u32,
    scale_factor_bits: u32,
}

#[derive(Clone)]
struct CachedTextRun {
    column: usize,
    cell_count: usize,
    column_count: usize,
    fixed_cell_starts: Option<Arc<[(usize, usize)]>>,
    foreground: Hsla,
    shaped: ShapedLine,
}

impl CachedTextRun {
    fn glyph_x(&self, cell_width: Pixels, byte_index: usize, shaped_x: Pixels) -> Pixels {
        let Some(cell_starts) = self.fixed_cell_starts.as_ref() else {
            return shaped_x;
        };
        let cell_index = cell_starts
            .partition_point(|(start, _)| *start <= byte_index)
            .saturating_sub(1);
        let extra_columns = cell_starts
            .get(cell_index)
            .map_or(0, |(_, column)| column.saturating_sub(cell_index));
        shaped_x + cell_width * extra_columns as f32
    }
}

#[derive(Clone)]
struct CachedRowDisplay {
    generation: u64,
    background_spans: Arc<[BackgroundSpan]>,
    text_runs: Arc<[CachedTextRun]>,
    decoration_spans: Arc<[DecorationSpan]>,
}

pub(crate) struct PreparedTerminalFrame {
    snapshot: Arc<TerminalRenderSnapshot>,
    rows: Vec<CachedRowDisplay>,
}

impl PreparedTerminalFrame {
    pub(crate) fn snapshot(&self) -> &TerminalRenderSnapshot {
        self.snapshot.as_ref()
    }
}

pub(crate) struct PreparedImeText {
    cursor_bounds: Bounds<Pixels>,
    background: Hsla,
    shaped: ShapedLine,
}

struct RendererShared {
    metrics_key: Option<FontMetricsKey>,
    metrics: Option<TerminalFontMetrics>,
    rows: HashMap<usize, CachedRowDisplay>,
    #[cfg(test)]
    shaping_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl RendererShared {
    fn new() -> Self {
        Self {
            metrics_key: None,
            metrics: None,
            rows: HashMap::new(),
            #[cfg(test)]
            shaping_hook: None,
        }
    }
}

#[derive(Default)]
struct TerminalDiagnostics {
    rebuilt_rows: AtomicU64,
    shaped_text_runs: AtomicU64,
    painted_text_runs: AtomicU64,
    painted_text_cells: AtomicU64,
    term_lock_nanos: AtomicU64,
    paint_nanos: AtomicU64,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalDiagnosticsSnapshot {
    pub rebuilt_rows: u64,
    pub shaped_text_runs: u64,
    pub painted_text_runs: u64,
    pub painted_text_cells: u64,
    pub term_lock_nanos: u64,
    pub paint_nanos: u64,
    pub bytes_read: u64,
    pub parser_batches: u64,
    pub read_batches_high_water: usize,
    pub queued_input_high_water: usize,
    pub queued_reply_high_water: usize,
    pub queued_command_high_water: usize,
    pub gpui_wakeups: u64,
}

/// GPUI painter for resolved terminal snapshots.
#[derive(Clone)]
pub struct TerminalRenderer {
    pub font_family: String,
    pub font_size: Pixels,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub line_height_multiplier: f32,
    pub palette: ColorPalette,
    pub cursor_thickness: f32,
    shared: Arc<Mutex<RendererShared>>,
    diagnostics: Arc<TerminalDiagnostics>,
}

impl TerminalRenderer {
    pub fn new(
        font_family: String,
        font_size: Pixels,
        line_height_multiplier: f32,
        palette: ColorPalette,
    ) -> Self {
        Self {
            font_family,
            font_size,
            cell_width: font_size * 0.6,
            cell_height: font_size * 1.4,
            line_height_multiplier,
            palette,
            cursor_thickness: 0.15,
            shared: Arc::new(Mutex::new(RendererShared::new())),
            diagnostics: Arc::new(TerminalDiagnostics::default()),
        }
    }

    pub fn measure_cell(&mut self, window: &mut Window) {
        self.ensure_metrics(window);
    }

    pub(crate) fn ensure_metrics(&mut self, window: &mut Window) -> TerminalFontMetrics {
        let font_size: f32 = self.font_size.into();
        let key = FontMetricsKey {
            family: self.font_family.clone(),
            font_size_bits: font_size.to_bits(),
            line_height_bits: self.line_height_multiplier.to_bits(),
            scale_factor_bits: window.scale_factor().to_bits(),
        };
        if let Some(metrics) = {
            let shared = self.shared.lock();
            (shared.metrics_key.as_ref() == Some(&key))
                .then_some(shared.metrics)
                .flatten()
        } {
            self.cell_width = metrics.cell_width;
            self.cell_height = metrics.cell_height;
            return metrics;
        }

        let run = TextRun {
            len: 1,
            font: self.font(TerminalFontStyle::default()),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line("M".into(), self.font_size, &[run], None);
        let cell_width = if shaped.width > px(0.0) {
            shaped.width
        } else {
            self.font_size * 0.6
        };
        let natural_height = shaped.ascent + shaped.descent;
        let cell_height = if natural_height > px(0.0) {
            natural_height * self.line_height_multiplier
        } else {
            self.font_size * 1.4
        };
        let metrics = TerminalFontMetrics {
            cell_width,
            cell_height,
            ascent: shaped.ascent,
            descent: shaped.descent,
            scale_factor: window.scale_factor(),
        };
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        let mut shared = self.shared.lock();
        shared.metrics_key = Some(key);
        shared.metrics = Some(metrics);
        shared.rows.clear();
        metrics
    }

    fn font(&self, style: TerminalFontStyle) -> Font {
        Font {
            family: self.font_family.clone().into(),
            features: FontFeatures::disable_ligatures(),
            fallbacks: None,
            weight: if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        }
    }

    pub(crate) fn invalidate_font(&self) {
        let mut shared = self.shared.lock();
        shared.metrics_key = None;
        shared.metrics = None;
        shared.rows.clear();
    }

    pub(crate) fn invalidate_palette(&self) {
        self.shared.lock().rows.clear();
    }

    #[cfg(test)]
    pub(crate) fn set_shaping_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        self.shared.lock().shaping_hook = hook;
    }

    #[cfg(test)]
    pub(crate) fn cached_first_run_glyph_x_for_index(
        &self,
        row_index: usize,
        byte_index: usize,
    ) -> Option<Pixels> {
        let shared = self.shared.lock();
        let run = shared.rows.get(&row_index)?.text_runs.first()?;
        let glyph = run
            .shaped
            .runs
            .iter()
            .flat_map(|font_run| font_run.glyphs.iter())
            .filter(|glyph| glyph.index >= byte_index)
            .min_by_key(|glyph| glyph.index)?;
        Some(run.glyph_x(self.cell_width, glyph.index, glyph.position.x))
    }

    fn prepared_row(
        &self,
        row_index: usize,
        row: &RenderableRow,
        window: &mut Window,
    ) -> CachedRowDisplay {
        if let Some(cached) = self
            .shared
            .lock()
            .rows
            .get(&row_index)
            .filter(|cached| cached.generation == row.generation)
            .cloned()
        {
            return cached;
        }

        let batches = text_batches(&row.cells);
        let mut text_runs = Vec::with_capacity(batches.len());
        #[cfg(test)]
        let shaping_hook = self.shared.lock().shaping_hook.clone();

        for batch in batches {
            #[cfg(test)]
            if let Some(hook) = shaping_hook.as_ref() {
                hook();
            }

            let TerminalTextBatch {
                column,
                cell_count,
                column_count,
                cell_starts,
                text,
                font_style,
                foreground,
                ..
            } = batch;
            let run = TextRun {
                len: text.len(),
                font: self.font(font_style),
                color: foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(text),
                self.font_size,
                &[run],
                Some(self.cell_width),
            );
            text_runs.push(CachedTextRun {
                column,
                cell_count,
                column_count,
                fixed_cell_starts: cell_starts.map(Into::into),
                foreground,
                shaped,
            });
        }

        self.diagnostics
            .rebuilt_rows
            .fetch_add(1, Ordering::Relaxed);
        self.diagnostics
            .shaped_text_runs
            .fetch_add(text_runs.len() as u64, Ordering::Relaxed);
        let cached = CachedRowDisplay {
            generation: row.generation,
            background_spans: Self::background_spans(row_index, &row.cells).into(),
            text_runs: text_runs.into(),
            decoration_spans: Self::decoration_spans_for_row(row_index, &row.cells).into(),
        };
        self.shared.lock().rows.insert(row_index, cached.clone());
        cached
    }

    pub(crate) fn prepare_frame(
        &self,
        snapshot: Arc<TerminalRenderSnapshot>,
        window: &mut Window,
    ) -> PreparedTerminalFrame {
        let rows = snapshot
            .rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| self.prepared_row(row_index, row, window))
            .collect();
        PreparedTerminalFrame { snapshot, rows }
    }

    pub(crate) fn record_term_lock(&self, nanos: u64) {
        self.diagnostics
            .term_lock_nanos
            .fetch_add(nanos, Ordering::Relaxed);
    }

    #[cfg(any(test, debug_assertions))]
    pub fn diagnostics_snapshot(&self) -> TerminalDiagnosticsSnapshot {
        TerminalDiagnosticsSnapshot {
            rebuilt_rows: self.diagnostics.rebuilt_rows.load(Ordering::Relaxed),
            shaped_text_runs: self.diagnostics.shaped_text_runs.load(Ordering::Relaxed),
            painted_text_runs: self.diagnostics.painted_text_runs.load(Ordering::Relaxed),
            painted_text_cells: self.diagnostics.painted_text_cells.load(Ordering::Relaxed),
            term_lock_nanos: self.diagnostics.term_lock_nanos.load(Ordering::Relaxed),
            paint_nanos: self.diagnostics.paint_nanos.load(Ordering::Relaxed),
            ..TerminalDiagnosticsSnapshot::default()
        }
    }

    #[cfg(any(test, debug_assertions))]
    pub fn reset_diagnostics(&self) {
        self.diagnostics.rebuilt_rows.store(0, Ordering::Relaxed);
        self.diagnostics
            .shaped_text_runs
            .store(0, Ordering::Relaxed);
        self.diagnostics
            .painted_text_runs
            .store(0, Ordering::Relaxed);
        self.diagnostics
            .painted_text_cells
            .store(0, Ordering::Relaxed);
        self.diagnostics.term_lock_nanos.store(0, Ordering::Relaxed);
        self.diagnostics.paint_nanos.store(0, Ordering::Relaxed);
    }

    fn background_spans(row: usize, cells: &[RenderableCell]) -> Vec<BackgroundSpan> {
        let mut spans = Vec::new();
        let mut current: Option<BackgroundSpan> = None;
        for cell in cells {
            let column = cell.point.column.0;
            match &mut current {
                Some(span) if span.color == cell.background && span.end_col == column => {
                    span.end_col += 1;
                }
                Some(span) => {
                    spans.push(*span);
                    *span = BackgroundSpan {
                        row,
                        start_col: column,
                        end_col: column + 1,
                        color: cell.background,
                    };
                }
                None => {
                    current = Some(BackgroundSpan {
                        row,
                        start_col: column,
                        end_col: column + 1,
                        color: cell.background,
                    });
                }
            }
        }
        if let Some(span) = current {
            spans.push(span);
        }
        spans
    }

    fn decoration_spans_for_row(row_index: usize, cells: &[RenderableCell]) -> Vec<DecorationSpan> {
        let mut spans = Vec::new();
        for kind in [
            RenderDecorationFlags::UNDERLINE,
            RenderDecorationFlags::DOUBLE_UNDERLINE,
            RenderDecorationFlags::UNDERCURL,
            RenderDecorationFlags::DOTTED_UNDERLINE,
            RenderDecorationFlags::DASHED_UNDERLINE,
            RenderDecorationFlags::STRIKEOUT,
        ] {
            let mut current: Option<DecorationSpan> = None;
            for cell in cells {
                if !cell.decorations.contains(kind) {
                    if let Some(span) = current.take() {
                        spans.push(span);
                    }
                    continue;
                }
                let start_col = cell.point.column.0;
                let end_col = start_col + cell.width.columns();
                match &mut current {
                    Some(span)
                        if span.color == cell.underline_color && start_col <= span.end_col =>
                    {
                        span.end_col = span.end_col.max(end_col);
                    }
                    Some(span) => {
                        spans.push(*span);
                        *span = DecorationSpan {
                            row: row_index,
                            start_col,
                            end_col,
                            color: cell.underline_color,
                            kind,
                        };
                    }
                    None => {
                        current = Some(DecorationSpan {
                            row: row_index,
                            start_col,
                            end_col,
                            color: cell.underline_color,
                            kind,
                        });
                    }
                }
            }
            if let Some(span) = current {
                spans.push(span);
            }
        }
        spans
    }

    #[cfg(test)]
    fn decoration_spans(rows: &[RenderableRow]) -> Vec<DecorationSpan> {
        rows.iter()
            .enumerate()
            .flat_map(|(row_index, row)| Self::decoration_spans_for_row(row_index, &row.cells))
            .collect()
    }

    pub(crate) fn ime_caret_offset(
        &self,
        text: &str,
        caret_utf16: usize,
        window: &mut Window,
    ) -> Pixels {
        let mut units = 0;
        let prefix = text
            .chars()
            .take_while(|character| {
                let next = units + character.len_utf16();
                if next > caret_utf16 {
                    false
                } else {
                    units = next;
                    true
                }
            })
            .collect::<String>();
        if prefix.is_empty() {
            return px(0.0);
        }
        let run = TextRun {
            len: prefix.len(),
            font: self.font(TerminalFontStyle::default()),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(prefix.into(), self.font_size, &[run], None)
            .width
    }

    pub(crate) fn cursor_bounds(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        cursor: RenderableCursor,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds {
            origin: Point {
                x: bounds.origin.x + padding.left + self.cell_width * cursor.point.column.0 as f32,
                y: bounds.origin.y + padding.top + self.cell_height * cursor.point.line as f32,
            },
            size: Size {
                width: self.cell_width * cursor.width.get() as f32,
                height: self.cell_height,
            },
        })
    }

    pub(crate) fn prepare_ime_text(
        &self,
        cursor_bounds: Bounds<Pixels>,
        snapshot: &TerminalRenderSnapshot,
        text: &str,
        window: &mut Window,
    ) -> Option<PreparedImeText> {
        if text.is_empty() {
            return None;
        }
        let run = TextRun {
            len: text.len(),
            font: self.font(TerminalFontStyle::default()),
            color: snapshot.default_foreground,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(snapshot.default_foreground),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text.into(), self.font_size, &[run], None);
        Some(PreparedImeText {
            cursor_bounds,
            background: snapshot.default_background,
            shaped,
        })
    }

    pub(crate) fn paint_ime_text(
        &self,
        prepared: &PreparedImeText,
        window: &mut Window,
        cx: &mut App,
    ) {
        let ime_bounds = Bounds {
            origin: prepared.cursor_bounds.origin,
            size: Size {
                width: prepared.shaped.width.max(self.cell_width),
                height: self.cell_height,
            },
        };
        window.paint_quad(quad(
            ime_bounds,
            px(0.0),
            prepared.background,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
        let base_height = self.cell_height / self.line_height_multiplier;
        let vertical_offset = (self.cell_height - base_height) / 2.0;
        let _ = prepared.shaped.paint(
            Point {
                x: prepared.cursor_bounds.origin.x,
                y: prepared.cursor_bounds.origin.y + vertical_offset,
            },
            self.cell_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    fn paint_fixed_cell_text_run(
        &self,
        origin: Point<Pixels>,
        text_run: &CachedTextRun,
        window: &mut Window,
    ) {
        let line_bounds = Bounds {
            origin,
            size: Size {
                width: self.cell_width * text_run.column_count as f32,
                height: self.cell_height,
            },
        };
        let baseline = (self.cell_height - text_run.shaped.ascent - text_run.shaped.descent) / 2.0
            + text_run.shaped.ascent;
        window.paint_layer(line_bounds, |window| {
            for font_run in &text_run.shaped.runs {
                for glyph in &font_run.glyphs {
                    let glyph_origin = Point {
                        x: origin.x
                            + text_run.glyph_x(self.cell_width, glyph.index, glyph.position.x),
                        y: origin.y + baseline + glyph.position.y,
                    };
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(
                            glyph_origin,
                            font_run.font_id,
                            glyph.id,
                            text_run.shaped.font_size,
                        );
                    } else {
                        let _ = window.paint_glyph(
                            glyph_origin,
                            font_run.font_id,
                            glyph.id,
                            text_run.shaped.font_size,
                            text_run.foreground,
                        );
                    }
                }
            }
        });
    }

    pub(crate) fn paint(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        show_scrollbar: bool,
        prepared: &PreparedTerminalFrame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let paint_started = Instant::now();
        let snapshot = prepared.snapshot();
        window.paint_quad(quad(
            bounds,
            px(0.0),
            snapshot.default_background,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));

        let origin = Point {
            x: bounds.origin.x + padding.left,
            y: bounds.origin.y + padding.top,
        };
        let base_height = self.cell_height / self.line_height_multiplier;
        let vertical_offset = (self.cell_height - base_height) / 2.0;

        let mut painted_text_runs = 0_u64;
        let mut painted_text_cells = 0_u64;
        for (row_index, row) in prepared.rows.iter().enumerate() {
            for span in row.background_spans.iter() {
                if span.color == snapshot.default_background {
                    continue;
                }
                window.paint_quad(quad(
                    Bounds {
                        origin: Point {
                            x: origin.x + self.cell_width * span.start_col as f32,
                            y: origin.y + self.cell_height * span.row as f32,
                        },
                        size: Size {
                            width: self.cell_width * (span.end_col - span.start_col) as f32,
                            height: self.cell_height,
                        },
                    },
                    px(0.0),
                    span.color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }

            for text_run in row.text_runs.iter() {
                let text_origin = Point {
                    x: origin.x + self.cell_width * text_run.column as f32,
                    y: origin.y + self.cell_height * row_index as f32 + vertical_offset,
                };
                if text_run.fixed_cell_starts.is_some() {
                    self.paint_fixed_cell_text_run(text_origin, text_run, window);
                } else {
                    let _ = text_run.shaped.paint(
                        text_origin,
                        self.cell_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                painted_text_runs += 1;
                painted_text_cells += text_run.cell_count as u64;
            }
        }

        self.paint_decorations(
            origin,
            prepared
                .rows
                .iter()
                .flat_map(|row| row.decoration_spans.iter()),
            window,
        );

        self.paint_cursor(bounds, padding, snapshot.cursor, window);
        if show_scrollbar {
            self.paint_scrollbar(bounds, padding, snapshot, window);
        }
        self.diagnostics
            .painted_text_runs
            .fetch_add(painted_text_runs, Ordering::Relaxed);
        self.diagnostics
            .painted_text_cells
            .fetch_add(painted_text_cells, Ordering::Relaxed);
        self.diagnostics.paint_nanos.fetch_add(
            paint_started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    fn paint_decorations<'a>(
        &self,
        origin: Point<Pixels>,
        spans: impl Iterator<Item = &'a DecorationSpan>,
        window: &mut Window,
    ) {
        let font_size: f32 = self.font_size.into();
        let thickness = px((font_size * 0.06).max(1.0));
        for span in spans {
            let x = origin.x + self.cell_width * span.start_col as f32;
            let width = self.cell_width * (span.end_col - span.start_col) as f32;
            let row_top = origin.y + self.cell_height * span.row as f32;
            let baseline = row_top + self.cell_height - thickness * 2.0;
            let strike_y = row_top + self.cell_height * 0.5;
            let paint_rect = |bounds: Bounds<Pixels>, window: &mut Window| {
                window.paint_quad(quad(
                    bounds,
                    px(0.0),
                    span.color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            };
            match span.kind {
                RenderDecorationFlags::UNDERLINE => paint_rect(
                    Bounds {
                        origin: Point { x, y: baseline },
                        size: Size {
                            width,
                            height: thickness,
                        },
                    },
                    window,
                ),
                RenderDecorationFlags::DOUBLE_UNDERLINE => {
                    for offset in [px(0.0), thickness * 2.0] {
                        paint_rect(
                            Bounds {
                                origin: Point {
                                    x,
                                    y: baseline - offset,
                                },
                                size: Size {
                                    width,
                                    height: thickness,
                                },
                            },
                            window,
                        );
                    }
                }
                RenderDecorationFlags::STRIKEOUT => paint_rect(
                    Bounds {
                        origin: Point { x, y: strike_y },
                        size: Size {
                            width,
                            height: thickness,
                        },
                    },
                    window,
                ),
                RenderDecorationFlags::DOTTED_UNDERLINE => {
                    let mut offset = px(0.0);
                    while offset < width {
                        paint_rect(
                            Bounds {
                                origin: Point {
                                    x: x + offset,
                                    y: baseline,
                                },
                                size: Size {
                                    width: thickness,
                                    height: thickness,
                                },
                            },
                            window,
                        );
                        offset += thickness * 2.0;
                    }
                }
                RenderDecorationFlags::DASHED_UNDERLINE => {
                    let dash = (self.cell_width * 0.5).max(thickness * 3.0);
                    let mut offset = px(0.0);
                    while offset < width {
                        paint_rect(
                            Bounds {
                                origin: Point {
                                    x: x + offset,
                                    y: baseline,
                                },
                                size: Size {
                                    width: dash.min(width - offset),
                                    height: thickness,
                                },
                            },
                            window,
                        );
                        offset += dash + thickness * 2.0;
                    }
                }
                RenderDecorationFlags::UNDERCURL => {
                    let segment = thickness * 2.0;
                    let mut offset = px(0.0);
                    let mut high = false;
                    while offset < width {
                        paint_rect(
                            Bounds {
                                origin: Point {
                                    x: x + offset,
                                    y: if high { baseline - thickness } else { baseline },
                                },
                                size: Size {
                                    width: segment.min(width - offset),
                                    height: thickness,
                                },
                            },
                            window,
                        );
                        high = !high;
                        offset += segment;
                    }
                }
                _ => {}
            }
        }
    }

    fn paint_cursor(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        cursor: RenderableCursor,
        window: &mut Window,
    ) {
        let Some(cursor_bounds) = self.cursor_bounds(bounds, padding, cursor) else {
            return;
        };
        let thickness = (self.cell_width * self.cursor_thickness.clamp(0.05, 1.0)).max(px(1.0));
        let geometry = match cursor.shape {
            CursorShape::Block | CursorShape::Hidden => return,
            CursorShape::Underline => Bounds {
                origin: Point {
                    x: cursor_bounds.origin.x,
                    y: cursor_bounds.origin.y + cursor_bounds.size.height - thickness,
                },
                size: Size {
                    width: cursor_bounds.size.width,
                    height: thickness,
                },
            },
            CursorShape::Beam => Bounds {
                origin: cursor_bounds.origin,
                size: Size {
                    width: thickness,
                    height: cursor_bounds.size.height,
                },
            },
            CursorShape::HollowBlock => {
                window.paint_quad(quad(
                    cursor_bounds,
                    px(0.0),
                    transparent_black(),
                    Edges::all(thickness),
                    cursor.cursor_color,
                    Default::default(),
                ));
                return;
            }
        };
        window.paint_quad(quad(
            geometry,
            px(0.0),
            cursor.cursor_color,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
    }

    fn paint_scrollbar(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        snapshot: &TerminalRenderSnapshot,
        window: &mut Window,
    ) {
        let Some(metrics) = TerminalScrollbarMetrics::from_rows(
            snapshot.history_size,
            snapshot.screen_lines,
            snapshot.display_offset,
        ) else {
            return;
        };
        let track_height = bounds.size.height - padding.top - padding.bottom;
        if track_height <= px(12.0) {
            return;
        }
        let track_width = px(3.0);
        let track_bounds = Bounds {
            origin: Point {
                x: bounds.origin.x + bounds.size.width - px(3.0) - track_width,
                y: bounds.origin.y + padding.top,
            },
            size: Size {
                width: track_width,
                height: track_height,
            },
        };
        window.paint_quad(quad(
            track_bounds,
            px(1.5),
            snapshot.default_foreground.alpha(0.06),
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
        let thumb_height = (track_height * metrics.thumb_height_fraction).min(track_height);
        window.paint_quad(quad(
            Bounds {
                origin: Point {
                    x: track_bounds.origin.x,
                    y: track_bounds.origin.y + track_height * metrics.thumb_top_fraction,
                },
                size: Size {
                    width: track_width,
                    height: thumb_height,
                },
            },
            px(1.5),
            snapshot.default_foreground.alpha(0.28),
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::content::RenderDamage;
    use super::*;
    use crate::test_support::TerminalFixture;
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};

    fn snapshot(fixture: &TerminalFixture) -> TerminalRenderSnapshot {
        fixture.terminal.with_term_mut(|term| {
            TerminalRenderSnapshot::build(
                term,
                &ColorPalette::default(),
                &RenderOverlayState::default(),
                true,
                true,
                true,
                &[],
                1,
            )
        })
    }

    #[test]
    fn render_snapshot_resolves_full_cell_semantics() {
        let mut fixture = TerminalFixture::new(16, 2);
        fixture.feed(b"\x1b[1;3;2;7;8;9;4mA");
        let snapshot = snapshot(&fixture);
        let cell = &snapshot.rows[0].cells[0];
        assert!(cell.font_style.bold);
        assert!(cell.font_style.italic);
        assert!(cell.font_style.dim);
        assert!(
            cell.text.is_empty(),
            "hidden text must preserve only its paint cell"
        );
        assert!(cell.decorations.contains(RenderDecorationFlags::UNDERLINE));
        assert!(cell.decorations.contains(RenderDecorationFlags::STRIKEOUT));
        assert_eq!(cell.width, TerminalCellWidth::Single);
    }

    #[test]
    fn render_snapshot_preserves_cjk_and_combining_clusters() {
        let mut fixture = TerminalFixture::new(16, 2);
        fixture.feed("提交e\u{301}".as_bytes());
        let snapshot = snapshot(&fixture);
        let cells = &snapshot.rows[0].cells;
        assert_eq!(cells[0].text.as_slice(), &['提']);
        assert_eq!(cells[0].width, TerminalCellWidth::Wide);
        assert_eq!(cells[1].width, TerminalCellWidth::Spacer);
        assert!(cells[1].text.is_empty());
        assert_eq!(cells[2].text.as_slice(), &['交']);
        assert_eq!(cells[2].width, TerminalCellWidth::Wide);
        assert_eq!(cells[3].width, TerminalCellWidth::Spacer);
        assert_eq!(cells[4].text.as_slice(), &['e', '\u{301}']);
    }

    #[test]
    fn block_cursor_covers_both_columns_of_a_wide_character() {
        let mut fixture = TerminalFixture::new(4, 1);
        fixture.feed("你".as_bytes());
        fixture.feed(b"\x1b[2D");

        let snapshot = snapshot(&fixture);
        assert_eq!(snapshot.cursor.point.column.0, 0);
        assert_eq!(snapshot.cursor.width.get(), 2);
        assert_eq!(snapshot.rows[0].cells[0].width, TerminalCellWidth::Wide);
        assert_eq!(snapshot.rows[0].cells[1].width, TerminalCellWidth::Spacer);
        assert_eq!(
            snapshot.rows[0].cells[0].background,
            snapshot.cursor.cursor_color
        );
        assert_eq!(
            snapshot.rows[0].cells[1].background,
            snapshot.cursor.cursor_color
        );
    }

    #[test]
    fn render_snapshot_applies_selection_and_cursor_precedence() {
        let mut fixture = TerminalFixture::new(4, 1);
        fixture.feed(b"ABC\x1b[2D");
        fixture.terminal.set_simple_selection(
            AlacPoint::new(Line(0), Column(0)),
            AlacPoint::new(Line(0), Column(2)),
        );
        let snapshot = snapshot(&fixture);
        let cell = &snapshot.rows[0].cells[1];
        assert!(cell.selected);
        assert_eq!(cell.background, snapshot.cursor.cursor_color);
        assert_eq!(cell.foreground, snapshot.cursor.text_color);
    }

    #[test]
    fn render_damage_rebuilds_only_changed_rows() {
        let mut fixture = TerminalFixture::new(4, 2);
        fixture.feed(b"A");
        let mut cache = TerminalRenderCache::default();
        assert_eq!(cache.merge(snapshot(&fixture)), 2);
        let first_generations = cache.row_generations();

        assert_eq!(cache.merge(snapshot(&fixture)), 0);
        assert_eq!(cache.row_generations(), first_generations);

        fixture.feed(b"B");
        assert_eq!(cache.merge(snapshot(&fixture)), 1);
        let changed_generations = cache.row_generations();
        assert_ne!(changed_generations[0], first_generations[0]);
        assert_eq!(changed_generations[1], first_generations[1]);
    }

    #[test]
    fn full_damage_diffs_final_rows_before_invalidating_row_cache() {
        let mut fixture = TerminalFixture::new(16, 3);
        fixture.feed(b"stable frame");
        let mut cache = TerminalRenderCache::default();
        assert_eq!(cache.merge(snapshot(&fixture)), 3);
        let first_generations = cache.row_generations();

        fixture.terminal.set_options(Default::default());
        let update = snapshot(&fixture);
        assert!(matches!(&update.damage, RenderDamage::Full));
        assert_eq!(cache.merge(update), 0);
        assert_eq!(cache.row_generations(), first_generations);
    }

    #[test]
    fn handed_off_frame_stays_immutable_while_cache_merges_next_update() {
        let mut fixture = TerminalFixture::new(4, 1);
        fixture.feed(b"A");
        let mut cache = TerminalRenderCache::default();
        cache.merge(snapshot(&fixture));
        let handed_off = cache.frame().unwrap();

        fixture.feed(b"B");
        assert_eq!(cache.merge(snapshot(&fixture)), 1);
        let current = cache.frame().unwrap();

        assert_eq!(handed_off.rows[0].cells[0].text.as_slice(), &['A']);
        assert!(handed_off.rows[0].cells[1].text.is_empty());
        assert_eq!(current.rows[0].cells[1].text.as_slice(), &['B']);
    }

    #[test]
    fn text_batches_merge_contiguous_same_style_cells() {
        let mut fixture = TerminalFixture::new(16, 1);
        fixture.feed(b"terminal-frame");
        let snapshot = snapshot(&fixture);

        let batches = text_batches(&snapshot.rows[0].cells);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].column, 0);
        assert_eq!(batches[0].cell_count, 14);
        assert_eq!(batches[0].text, "terminal-frame");
    }

    #[test]
    fn text_batches_split_at_style_and_cell_width_changes() {
        let mut fixture = TerminalFixture::new(8, 1);
        fixture.feed(b"A\x1b[31mB");
        fixture.feed("界C".as_bytes());
        let snapshot = snapshot(&fixture);

        let batches = text_batches(&snapshot.rows[0].cells);

        assert_eq!(batches.len(), 4);
        assert_eq!(batches[0].text, "A");
        assert_eq!(batches[1].text, "B");
        assert_eq!(batches[2].text, "界");
        assert_eq!(batches[2].cell_columns, 2);
        assert_eq!(batches[3].column, 4);
        assert_eq!(batches[3].text, "C");
    }

    #[test]
    fn text_batches_merge_contiguous_wide_cells_across_spacers() {
        let mut fixture = TerminalFixture::new(16, 1);
        fixture.feed("提交中文".as_bytes());
        let snapshot = snapshot(&fixture);

        let batches = text_batches(&snapshot.rows[0].cells);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].text, "提交中文");
        assert_eq!(batches[0].cell_count, 4);
        assert_eq!(batches[0].column_count, 8);
        assert_eq!(batches[0].cell_columns, 2);
    }

    #[test]
    fn text_batches_keep_combining_marks_with_their_base_cell() {
        let mut fixture = TerminalFixture::new(8, 1);
        fixture.feed("e\u{301}x".as_bytes());
        let snapshot = snapshot(&fixture);

        let batches = text_batches(&snapshot.rows[0].cells);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].text, "e\u{301}x");
        assert_eq!(batches[0].cell_count, 2);
    }

    #[test]
    fn decoration_spans_merge_and_cover_wide_cells() {
        let mut fixture = TerminalFixture::new(8, 1);
        fixture.feed(b"\x1b[4:2m");
        fixture.feed("界界".as_bytes());
        let snapshot = snapshot(&fixture);
        let spans = TerminalRenderer::decoration_spans(&snapshot.rows)
            .into_iter()
            .filter(|span| span.kind == RenderDecorationFlags::DOUBLE_UNDERLINE)
            .collect::<Vec<_>>();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start_col, 0);
        assert_eq!(spans[0].end_col, 4);
    }

    #[test]
    fn wide_character_backgrounds_cover_spacers_and_merge_contiguously() {
        let mut fixture = TerminalFixture::new(4, 1);
        fixture.feed("界".as_bytes());
        let snapshot = snapshot(&fixture);
        let spans = TerminalRenderer::background_spans(0, &snapshot.rows[0].cells);
        assert_eq!(spans[0].start_col, 0);
        assert_eq!(spans[0].end_col, 2);
    }
}
