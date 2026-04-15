use super::*;

pub(crate) fn shape_terminal_glyph_cells(
    glyph_cells: &[TerminalPaintGlyphCell],
    glyph_cache: &mut TerminalGlyphLayoutCache,
    glyph_cache_hits: &mut usize,
    glyph_cache_misses: &mut usize,
    window: &mut Window,
) -> Vec<TerminalShapedGlyphCell> {
    glyph_cells
        .iter()
        .enumerate()
        .map(|(input_cell_index, cell)| {
            let key = TerminalGlyphCacheKey {
                text: cell.text.to_string(),
                bold: cell.bold,
                italic: cell.italic,
                strikethrough: cell.strikethrough,
            };
            if let Some(line) = glyph_cache.glyphs.get(&key) {
                *glyph_cache_hits += 1;
                return TerminalShapedGlyphCell {
                    input_cell_index,
                    line: line.clone(),
                };
            }

            let text = cell.text.clone();
            let style =
                terminal_glyph_shape_style(text.len(), cell.bold, cell.italic, cell.strikethrough);
            let line = window.text_system().shape_line(
                text,
                px(TERMINAL_FONT_SIZE),
                std::slice::from_ref(&style),
                Some(px(TERMINAL_CELL_WIDTH)),
            );
            glyph_cache.glyphs.insert(key, line.clone());
            *glyph_cache_misses += 1;
            TerminalShapedGlyphCell {
                input_cell_index,
                line,
            }
        })
        .collect()
}

pub(crate) fn terminal_cached_paint_row(
    row_index: usize,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
    default_bg: Rgba,
) -> TerminalCachedPaintRow {
    let background_runs = terminal_background_runs(row_index as u16, row, snapshot, default_bg);
    let mut glyph_cells = Vec::new();

    for (col_index, cell) in row.cells.iter().enumerate() {
        if cell.width > 0 && !cell.invisible && !cell.text.is_empty() && cell.text != " " {
            let (fg, _) = terminal_effective_cell_colors(cell, snapshot);
            glyph_cells.push(TerminalPaintGlyphCell {
                row_index,
                col_index,
                text: SharedString::from(cell.text.clone()),
                color: Hsla::from(fg),
                bold: cell.bold,
                italic: cell.italic,
                underline: cell.underline,
                strikethrough: cell.strikethrough,
                overline: cell.overline,
                cell_width: cell.width,
            });
        }
    }

    TerminalCachedPaintRow {
        row_input: TerminalPaintRowInput {
            default_bg,
            background_runs,
        },
        glyph_cells,
    }
}

pub(crate) fn terminal_row_views_for_input(
    input: &TerminalGridPaintInput,
    glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    render_cache: &mut TerminalRenderCache,
    cx: &mut Context<OcttyApp>,
) -> Vec<AnyView> {
    let Some(cache) = render_cache.sessions.get_mut(&input.session_id) else {
        return Vec::new();
    };
    if cache.row_views.len() != input.rows_data.len() {
        cache.row_views = vec![None; input.rows_data.len()];
    }

    let mut views = Vec::with_capacity(input.rows_data.len());
    for row_index in 0..input.rows_data.len() {
        let row = terminal_row_view_payload(input, cache, row_index);
        let view = if let Some(view) = cache.row_views[row_index].as_ref() {
            if input
                .rebuilt_row_flags
                .get(row_index)
                .copied()
                .unwrap_or(true)
            {
                let _ = view.update(cx, |view, cx| {
                    view.cols = input.cols;
                    view.row_input = row.row_input;
                    view.glyph_cells = row.glyph_cells;
                    view.glyph_cache = glyph_cache.clone();
                    cx.notify();
                });
            }
            view.clone()
        } else {
            let view = cx.new(|_| TerminalRowView {
                cols: input.cols,
                row_input: row.row_input,
                glyph_cells: row.glyph_cells,
                glyph_cache: glyph_cache.clone(),
            });
            cache.row_views[row_index] = Some(view.clone());
            view
        };

        let any_view: AnyView = view.into();
        views.push(any_view);
    }

    views
}

pub(crate) fn terminal_row_view_payload(
    input: &TerminalGridPaintInput,
    cache: &TerminalRenderGridCache,
    row_index: usize,
) -> TerminalCachedPaintRow {
    if let Some(row) = cache.rows_data.get(row_index).and_then(Option::as_ref) {
        return row.clone();
    }

    TerminalCachedPaintRow {
        row_input: input.rows_data[row_index].clone(),
        glyph_cells: input
            .glyph_cells
            .iter()
            .filter(|cell| cell.row_index == row_index)
            .cloned()
            .collect(),
    }
}

impl Render for TerminalRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let cols = self.cols;
        let row_input = self.row_input.clone();
        let glyph_cells = self.glyph_cells.clone();
        let glyph_cache = self.glyph_cache.clone();
        let width = TERMINAL_CELL_WIDTH * cols as f32;

        canvas(
            move |_bounds, window, _cx| {
                let mut cache = glyph_cache.borrow_mut();
                let mut glyph_cache_hits = 0usize;
                let mut glyph_cache_misses = 0usize;
                let shaped_glyph_cells = shape_terminal_glyph_cells(
                    &glyph_cells,
                    &mut cache,
                    &mut glyph_cache_hits,
                    &mut glyph_cache_misses,
                    window,
                );
                TerminalRowPaintSurface {
                    row_input,
                    glyph_cells,
                    shaped_glyph_cells,
                }
            },
            move |bounds, surface, window, cx| {
                let paint_started_at = Instant::now();
                paint_terminal_row_surface(bounds, &surface, window, cx);
                let paint_micros = duration_micros(paint_started_at.elapsed());
                record_terminal_row_paint_profile(&surface, cols, paint_micros);
            },
        )
        .w(px(width))
        .h(px(TERMINAL_CELL_HEIGHT))
        .overflow_hidden()
    }
}

pub(crate) fn terminal_glyph_shape_style(
    len: usize,
    bold: bool,
    italic: bool,
    strikethrough: bool,
) -> TextRun {
    let mut font = terminal_font();
    if bold {
        font = font.bold();
    }
    if italic {
        font = font.italic();
    }
    TextRun {
        len,
        font,
        color: Hsla::from(rgb(0xffffff)),
        background_color: None,
        underline: None,
        strikethrough: strikethrough.then_some(Default::default()),
    }
}

pub(crate) fn paint_terminal_row_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: &TerminalRowPaintSurface,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, surface.row_input.default_bg));

    for run in &surface.row_input.background_runs {
        let origin = point(
            bounds.origin.x + px(run.start_col as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y,
        );
        let size = size(
            px(run.cell_count as f32 * TERMINAL_CELL_WIDTH),
            px(TERMINAL_CELL_HEIGHT),
        );
        window.paint_quad(fill(Bounds { origin, size }, run.color));
    }

    for shaped_cell in surface.shaped_glyph_cells.iter() {
        let cell = &surface.glyph_cells[shaped_cell.input_cell_index];
        let origin = point(
            bounds.origin.x + px(cell.col_index as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y,
        );
        let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
    }
}

pub(crate) fn paint_terminal_cursor_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: &TerminalCursorPaintSurface,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = bounds.origin;
    let cell_bounds = Bounds {
        origin,
        size: bounds.size,
    };
    window.paint_quad(fill(cell_bounds, surface.cursor.background));

    if let (Some(cell), Some(shaped_cell)) = (
        surface.cursor.glyph_cell.as_ref(),
        surface.shaped_glyph_cells.first(),
    ) {
        let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
    }
}

pub(crate) fn paint_terminal_selection_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: &TerminalSelectionPaintSurface,
    window: &mut Window,
) {
    let color = rgba(0x4f86f733);
    for run in &surface.runs {
        if run.end_col <= run.start_col {
            continue;
        }
        let origin = point(
            bounds.origin.x + px(f32::from(run.start_col) * TERMINAL_CELL_WIDTH),
            bounds.origin.y + px(f32::from(run.row) * TERMINAL_CELL_HEIGHT),
        );
        let size = size(
            px(f32::from(run.end_col - run.start_col) * TERMINAL_CELL_WIDTH),
            px(TERMINAL_CELL_HEIGHT),
        );
        window.paint_quad(fill(Bounds { origin, size }, color));
    }
}

pub(crate) fn paint_terminal_full_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: TerminalFullPaintSurface,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, surface.input.default_bg));

    for (row_index, row) in surface.input.rows_data.iter().enumerate() {
        let row_top = bounds.origin.y + px(row_index as f32 * TERMINAL_CELL_HEIGHT);
        for run in &row.background_runs {
            let origin = point(
                bounds.origin.x + px(run.start_col as f32 * TERMINAL_CELL_WIDTH),
                row_top,
            );
            let size = size(
                px(run.cell_count as f32 * TERMINAL_CELL_WIDTH),
                px(TERMINAL_CELL_HEIGHT),
            );
            window.paint_quad(fill(Bounds { origin, size }, run.color));
        }
    }

    for shaped_cell in surface.shaped_glyph_cells.iter() {
        let cell = &surface.input.glyph_cells[shaped_cell.input_cell_index];
        let origin = point(
            bounds.origin.x + px(cell.col_index as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y + px(cell.row_index as f32 * TERMINAL_CELL_HEIGHT),
        );
        let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
    }

    if let Some(cursor) = surface.input.cursor.as_ref() {
        let origin = point(
            bounds.origin.x + px(cursor.col_index as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y + px(cursor.row_index as f32 * TERMINAL_CELL_HEIGHT),
        );
        let cell_bounds = Bounds {
            origin,
            size: size(
                px(TERMINAL_CELL_WIDTH * f32::from(cursor.cell_width.max(1))),
                px(TERMINAL_CELL_HEIGHT),
            ),
        };
        window.paint_quad(fill(cell_bounds, cursor.background));
        if let (Some(cell), Some(shaped_cell)) = (
            cursor.glyph_cell.as_ref(),
            surface.shaped_cursor_glyph_cells.first(),
        ) {
            let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
        }
    }
}

pub(crate) fn paint_terminal_glyph_cell(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
    line: &ShapedLine,
    window: &mut Window,
    _cx: &mut App,
) -> gpui::Result<()> {
    let cell_bounds = terminal_glyph_cell_bounds(origin, cell);
    window.with_content_mask(
        Some(gpui::ContentMask {
            bounds: cell_bounds,
        }),
        |window| {
            let padding_top = (px(TERMINAL_CELL_HEIGHT) - line.ascent - line.descent) / 2.0;
            let baseline_offset = point(px(0.0), padding_top + line.ascent);
            let mut glyph_origin = origin;
            let mut prev_glyph_position = gpui::Point::default();

            for run in &line.runs {
                for glyph in &run.glyphs {
                    glyph_origin.x += glyph.position.x - prev_glyph_position.x;
                    prev_glyph_position = glyph.position;

                    if glyph.is_emoji {
                        window.paint_emoji(
                            glyph_origin + baseline_offset,
                            run.font_id,
                            glyph.id,
                            line.font_size,
                        )?;
                    } else {
                        window.paint_glyph(
                            glyph_origin + baseline_offset,
                            run.font_id,
                            glyph.id,
                            line.font_size,
                            cell.color,
                        )?;
                    }
                }
            }
            paint_terminal_cell_decorations(origin, cell, window);
            Ok(())
        },
    )
}

pub(crate) fn terminal_glyph_cell_bounds(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
) -> Bounds<gpui::Pixels> {
    Bounds {
        origin,
        size: size(
            px(TERMINAL_CELL_WIDTH * f32::from(cell.cell_width.max(1))),
            px(TERMINAL_CELL_HEIGHT),
        ),
    }
}

pub(crate) fn paint_terminal_cell_decorations(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
    window: &mut Window,
) {
    let width = px(TERMINAL_CELL_WIDTH * f32::from(cell.cell_width.max(1)));
    let thickness = px(1.0);
    if cell.overline {
        window.paint_quad(fill(
            Bounds {
                origin,
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
    if cell.strikethrough {
        window.paint_quad(fill(
            Bounds {
                origin: point(origin.x, origin.y + px(TERMINAL_CELL_HEIGHT * 0.5)),
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
    if cell.underline {
        window.paint_quad(fill(
            Bounds {
                origin: point(origin.x, origin.y + px(TERMINAL_CELL_HEIGHT - 2.0)),
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
}

pub(crate) fn terminal_background_runs(
    _row_index: u16,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
    default_bg: Rgba,
) -> Vec<TerminalPaintBackgroundRun> {
    let mut runs = Vec::new();
    let mut active: Option<TerminalPaintBackgroundRun> = None;

    for (col, cell) in row.cells.iter().enumerate() {
        let (_, bg) = terminal_effective_cell_colors(cell, snapshot);
        let bg = (bg != default_bg).then_some(bg);

        match (&mut active, bg) {
            (Some(run), Some(bg)) if run.color == bg && run.start_col + run.cell_count == col => {
                run.cell_count += 1;
            }
            (Some(_), Some(bg)) => {
                runs.push(active.take().expect("checked above"));
                active = Some(TerminalPaintBackgroundRun {
                    start_col: col,
                    cell_count: 1,
                    color: bg,
                });
            }
            (None, Some(bg)) => {
                active = Some(TerminalPaintBackgroundRun {
                    start_col: col,
                    cell_count: 1,
                    color: bg,
                });
            }
            (Some(_), None) => {
                runs.push(active.take().expect("checked above"));
            }
            (None, None) => {}
        }
    }

    if let Some(run) = active {
        runs.push(run);
    }

    runs
}

pub(crate) fn terminal_effective_cell_colors(
    cell: &octty_term::live::TerminalCellSnapshot,
    snapshot: &TerminalGridSnapshot,
) -> (Rgba, Rgba) {
    let mut fg = cell
        .fg
        .map(terminal_rgb_to_rgba)
        .unwrap_or_else(|| terminal_rgb_to_rgba(snapshot.default_fg));
    let mut bg = cell
        .bg
        .map(terminal_rgb_to_rgba)
        .unwrap_or_else(|| terminal_rgb_to_rgba(snapshot.default_bg));
    if cell.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.faint {
        fg = terminal_dim_color(fg, bg);
    }
    (fg, bg)
}

pub(crate) fn terminal_paint_cursor(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
) -> Option<TerminalPaintCursor> {
    let cursor = snapshot.cursor.as_ref().filter(|cursor| {
        cursor.visible && cursor.row < snapshot.rows && cursor.col < snapshot.cols
    })?;
    let row = snapshot.rows_data.get(cursor.row as usize)?;
    let cell = row.cells.get(cursor.col as usize)?;
    let glyph_cell =
        (cell.width > 0 && !cell.invisible && !cell.text.is_empty() && cell.text != " ").then(
            || TerminalPaintGlyphCell {
                row_index: cursor.row as usize,
                col_index: cursor.col as usize,
                text: SharedString::from(cell.text.clone()),
                cell_width: cell.width,
                color: Hsla::from(default_bg),
                bold: cell.bold,
                italic: cell.italic,
                underline: cell.underline,
                strikethrough: cell.strikethrough,
                overline: cell.overline,
            },
        );

    Some(TerminalPaintCursor {
        row_index: cursor.row as usize,
        col_index: cursor.col as usize,
        cell_width: cell.width.max(1),
        background: default_fg,
        glyph_cell,
    })
}
