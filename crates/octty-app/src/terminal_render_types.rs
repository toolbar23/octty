struct TerminalGridPaintInput {
    session_id: String,
    cols: u16,
    rows: u16,
    default_bg: Rgba,
    rows_data: Vec<TerminalPaintRowInput>,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    cursor: Option<TerminalPaintCursor>,
    dirty_rows: usize,
    dirty_cells: usize,
    rebuilt_rows: usize,
    reused_rows: usize,
    repaint_backgrounds: usize,
    rebuilt_row_flags: Vec<bool>,
}

#[derive(Clone)]
struct TerminalPaintRowInput {
    default_bg: Rgba,
    background_runs: Vec<TerminalPaintBackgroundRun>,
}

#[derive(Clone)]
struct TerminalPaintGlyphCell {
    row_index: usize,
    col_index: usize,
    text: SharedString,
    cell_width: u8,
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    overline: bool,
}

#[derive(Clone)]
struct TerminalPaintBackgroundRun {
    start_col: usize,
    cell_count: usize,
    color: Rgba,
}

#[derive(Clone)]
struct TerminalPaintCursor {
    row_index: usize,
    col_index: usize,
    cell_width: u8,
    background: Rgba,
    glyph_cell: Option<TerminalPaintGlyphCell>,
}

struct TerminalRowPaintSurface {
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
}

struct TerminalCursorPaintSurface {
    cursor: TerminalPaintCursor,
    shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
}

struct TerminalSelectionPaintSurface {
    runs: Vec<TerminalSelectionRun>,
}

struct TerminalFullPaintSurface {
    input: TerminalGridPaintInput,
    shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
    shaped_cursor_glyph_cells: Vec<TerminalShapedGlyphCell>,
    glyph_cache_hits: usize,
    glyph_cache_misses: usize,
}

struct TerminalShapedGlyphCell {
    input_cell_index: usize,
    line: ShapedLine,
}

#[derive(Default)]
struct TerminalGlyphLayoutCache {
    glyphs: HashMap<TerminalGlyphCacheKey, ShapedLine>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TerminalGlyphCacheKey {
    text: String,
    bold: bool,
    italic: bool,
    strikethrough: bool,
}

#[derive(Default)]
struct TerminalRenderCache {
    sessions: HashMap<String, TerminalRenderGridCache>,
}

struct TerminalRenderGridCache {
    cols: u16,
    rows: u16,
    default_fg: Rgba,
    default_bg: Rgba,
    rows_data: Vec<Option<TerminalCachedPaintRow>>,
    row_views: Vec<Option<Entity<TerminalRowView>>>,
    interaction: Rc<RefCell<TerminalGridInteractionState>>,
}

#[derive(Default)]
struct TerminalGridInteractionState {
    bounds: Option<Bounds<Pixels>>,
}

#[derive(Clone)]
struct TerminalCachedPaintRow {
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
}

struct TerminalRowView {
    cols: u16,
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TerminalRenderProfileSample {
    build_micros: u64,
    shape_micros: u64,
    paint_micros: u64,
    rows: u16,
    cols: u16,
    glyph_cells: usize,
    glyph_cache_hits: usize,
    glyph_cache_misses: usize,
    background_runs: usize,
    text_bytes: usize,
    dirty_rows: usize,
    dirty_cells: usize,
    rebuilt_rows: usize,
    reused_rows: usize,
    repaint_backgrounds: usize,
    painted_rows: usize,
    submitted_glyphs: usize,
    submitted_backgrounds: usize,
}

#[derive(Default)]
struct TerminalRenderProfiler {
    build_micros: VecDeque<u64>,
    shape_micros: VecDeque<u64>,
    paint_micros: VecDeque<u64>,
    glyph_cells: VecDeque<u64>,
    glyph_cache_hits: VecDeque<u64>,
    glyph_cache_misses: VecDeque<u64>,
    background_runs: VecDeque<u64>,
    text_bytes: VecDeque<u64>,
    dirty_rows: VecDeque<u64>,
    dirty_cells: VecDeque<u64>,
    rebuilt_rows: VecDeque<u64>,
    reused_rows: VecDeque<u64>,
    repaint_backgrounds: VecDeque<u64>,
    painted_rows: VecDeque<u64>,
    submitted_glyphs: VecDeque<u64>,
    submitted_backgrounds: VecDeque<u64>,
    last_report_at: Option<Instant>,
}
