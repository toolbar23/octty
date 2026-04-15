use super::*;

pub(crate) struct TerminalGridPaintInput {
    pub(crate) session_id: String,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) default_bg: Rgba,
    pub(crate) rows_data: Vec<TerminalPaintRowInput>,
    pub(crate) glyph_cells: Vec<TerminalPaintGlyphCell>,
    pub(crate) cursor: Option<TerminalPaintCursor>,
    pub(crate) dirty_rows: usize,
    pub(crate) dirty_cells: usize,
    pub(crate) rebuilt_rows: usize,
    pub(crate) reused_rows: usize,
    pub(crate) repaint_backgrounds: usize,
    pub(crate) rebuilt_row_flags: Vec<bool>,
}

#[derive(Clone)]
pub(crate) struct TerminalPaintRowInput {
    pub(crate) default_bg: Rgba,
    pub(crate) background_runs: Vec<TerminalPaintBackgroundRun>,
}

#[derive(Clone)]
pub(crate) struct TerminalPaintGlyphCell {
    pub(crate) row_index: usize,
    pub(crate) col_index: usize,
    pub(crate) text: SharedString,
    pub(crate) cell_width: u8,
    pub(crate) color: Hsla,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) overline: bool,
}

#[derive(Clone)]
pub(crate) struct TerminalPaintBackgroundRun {
    pub(crate) start_col: usize,
    pub(crate) cell_count: usize,
    pub(crate) color: Rgba,
}

#[derive(Clone)]
pub(crate) struct TerminalPaintCursor {
    pub(crate) row_index: usize,
    pub(crate) col_index: usize,
    pub(crate) cell_width: u8,
    pub(crate) background: Rgba,
    pub(crate) glyph_cell: Option<TerminalPaintGlyphCell>,
}

pub(crate) struct TerminalRowPaintSurface {
    pub(crate) row_input: TerminalPaintRowInput,
    pub(crate) glyph_cells: Vec<TerminalPaintGlyphCell>,
    pub(crate) shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
}

pub(crate) struct TerminalCursorPaintSurface {
    pub(crate) cursor: TerminalPaintCursor,
    pub(crate) shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
}

pub(crate) struct TerminalSelectionPaintSurface {
    pub(crate) runs: Vec<TerminalSelectionRun>,
}

pub(crate) struct TerminalFullPaintSurface {
    pub(crate) input: TerminalGridPaintInput,
    pub(crate) shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
    pub(crate) shaped_cursor_glyph_cells: Vec<TerminalShapedGlyphCell>,
    pub(crate) glyph_cache_hits: usize,
    pub(crate) glyph_cache_misses: usize,
}

pub(crate) struct TerminalShapedGlyphCell {
    pub(crate) input_cell_index: usize,
    pub(crate) line: ShapedLine,
}

#[derive(Default)]
pub(crate) struct TerminalGlyphLayoutCache {
    pub(crate) glyphs: HashMap<TerminalGlyphCacheKey, ShapedLine>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TerminalGlyphCacheKey {
    pub(crate) text: String,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) strikethrough: bool,
}

#[derive(Default)]
pub(crate) struct TerminalRenderCache {
    pub(crate) sessions: HashMap<String, TerminalRenderGridCache>,
}

pub(crate) struct TerminalRenderGridCache {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) default_fg: Rgba,
    pub(crate) default_bg: Rgba,
    pub(crate) rows_data: Vec<Option<TerminalCachedPaintRow>>,
    pub(crate) row_views: Vec<Option<Entity<TerminalRowView>>>,
    pub(crate) interaction: Rc<RefCell<TerminalGridInteractionState>>,
}

#[derive(Default)]
pub(crate) struct TerminalGridInteractionState {
    pub(crate) bounds: Option<Bounds<Pixels>>,
}

#[derive(Clone)]
pub(crate) struct TerminalCachedPaintRow {
    pub(crate) row_input: TerminalPaintRowInput,
    pub(crate) glyph_cells: Vec<TerminalPaintGlyphCell>,
}

pub(crate) struct TerminalRowView {
    pub(crate) cols: u16,
    pub(crate) row_input: TerminalPaintRowInput,
    pub(crate) glyph_cells: Vec<TerminalPaintGlyphCell>,
    pub(crate) glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TerminalRenderProfileSample {
    pub(crate) build_micros: u64,
    pub(crate) shape_micros: u64,
    pub(crate) paint_micros: u64,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) glyph_cells: usize,
    pub(crate) glyph_cache_hits: usize,
    pub(crate) glyph_cache_misses: usize,
    pub(crate) background_runs: usize,
    pub(crate) text_bytes: usize,
    pub(crate) dirty_rows: usize,
    pub(crate) dirty_cells: usize,
    pub(crate) rebuilt_rows: usize,
    pub(crate) reused_rows: usize,
    pub(crate) repaint_backgrounds: usize,
    pub(crate) painted_rows: usize,
    pub(crate) submitted_glyphs: usize,
    pub(crate) submitted_backgrounds: usize,
}

#[derive(Default)]
pub(crate) struct TerminalRenderProfiler {
    pub(crate) build_micros: VecDeque<u64>,
    pub(crate) shape_micros: VecDeque<u64>,
    pub(crate) paint_micros: VecDeque<u64>,
    pub(crate) glyph_cells: VecDeque<u64>,
    pub(crate) glyph_cache_hits: VecDeque<u64>,
    pub(crate) glyph_cache_misses: VecDeque<u64>,
    pub(crate) background_runs: VecDeque<u64>,
    pub(crate) text_bytes: VecDeque<u64>,
    pub(crate) dirty_rows: VecDeque<u64>,
    pub(crate) dirty_cells: VecDeque<u64>,
    pub(crate) rebuilt_rows: VecDeque<u64>,
    pub(crate) reused_rows: VecDeque<u64>,
    pub(crate) repaint_backgrounds: VecDeque<u64>,
    pub(crate) painted_rows: VecDeque<u64>,
    pub(crate) submitted_glyphs: VecDeque<u64>,
    pub(crate) submitted_backgrounds: VecDeque<u64>,
    pub(crate) last_report_at: Option<Instant>,
}
