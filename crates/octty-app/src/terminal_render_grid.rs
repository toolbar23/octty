fn render_terminal_grid(
    live_key: String,
    snapshot: &TerminalGridSnapshot,
    selection: Option<&TerminalSelection>,
    default_fg: Rgba,
    default_bg: Rgba,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let build_started_at = Instant::now();
    let input = terminal_paint_input(
        snapshot,
        default_fg,
        default_bg,
        &mut terminal_render_cache.borrow_mut(),
    );
    let build_micros = duration_micros(build_started_at.elapsed());
    let width = TERMINAL_CELL_WIDTH * input.cols as f32;
    let height = TERMINAL_CELL_HEIGHT * input.rows as f32;
    let interaction =
        terminal_grid_interaction_state(&input.session_id, &mut terminal_render_cache.borrow_mut());

    if terminal_prefers_full_canvas(&input) {
        clear_terminal_row_views(&input.session_id, &mut terminal_render_cache.borrow_mut());
        return render_terminal_full_canvas(
            input,
            terminal_glyph_cache,
            build_micros,
            width,
            height,
        );
    }

    let row_views = {
        let mut render_cache = terminal_render_cache.borrow_mut();
        terminal_row_views_for_input(&input, terminal_glyph_cache.clone(), &mut render_cache, cx)
    };
    record_terminal_render_build_profile(&input, build_micros);
    let cursor = input.cursor.clone();

    let mut grid = div()
        .relative()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .children(row_views);
    if let Some(cursor) = cursor {
        grid = grid.child(render_terminal_cursor_overlay(cursor, terminal_glyph_cache));
    }
    grid = grid.child(render_terminal_selection_layer(
        live_key,
        selection.cloned(),
        input.cols,
        input.rows,
        interaction,
        cx,
    ));
    grid
}

fn terminal_paint_input(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
    render_cache: &mut TerminalRenderCache,
) -> TerminalGridPaintInput {
    let mut rows_data = Vec::with_capacity(snapshot.rows_data.len());
    let mut glyph_cells = Vec::new();
    let cache = render_cache
        .sessions
        .entry(snapshot.session_id.clone())
        .or_insert_with(|| TerminalRenderGridCache {
            cols: snapshot.cols,
            rows: snapshot.rows,
            default_fg,
            default_bg,
            rows_data: Vec::new(),
            row_views: Vec::new(),
            interaction: Rc::new(RefCell::new(TerminalGridInteractionState::default())),
        });
    let cache_invalid = cache.cols != snapshot.cols
        || cache.rows != snapshot.rows
        || cache.default_fg != default_fg
        || cache.default_bg != default_bg
        || cache.rows_data.len() != snapshot.rows_data.len();
    if cache_invalid {
        cache.cols = snapshot.cols;
        cache.rows = snapshot.rows;
        cache.default_fg = default_fg;
        cache.default_bg = default_bg;
        cache.rows_data = vec![None; snapshot.rows_data.len()];
        cache.row_views = vec![None; snapshot.rows_data.len()];
    }

    let mut dirty_row_flags = vec![cache_invalid || snapshot.damage.full; snapshot.rows_data.len()];
    for row in snapshot.damage.rows.iter().copied() {
        if let Some(flag) = dirty_row_flags.get_mut(row as usize) {
            *flag = true;
        }
    }

    let mut rebuilt_rows = 0usize;
    let mut reused_rows = 0usize;
    let mut repaint_backgrounds = 0usize;

    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        let rebuild_row = dirty_row_flags[row_index] || cache.rows_data[row_index].is_none();
        let cached_row = if rebuild_row {
            rebuilt_rows += 1;
            let cached_row = terminal_cached_paint_row(row_index, row, snapshot, default_bg);
            cache.rows_data[row_index] = Some(cached_row.clone());
            cached_row
        } else {
            reused_rows += 1;
            cache.rows_data[row_index]
                .as_ref()
                .expect("checked above")
                .clone()
        };

        if rebuild_row {
            glyph_cells.extend(cached_row.glyph_cells.iter().cloned());
            repaint_backgrounds += terminal_row_background_submission_count(&cached_row.row_input);
        }
        rows_data.push(cached_row.row_input);
    }

    TerminalGridPaintInput {
        session_id: snapshot.session_id.clone(),
        cols: snapshot.cols,
        rows: snapshot.rows,
        default_bg,
        rows_data,
        glyph_cells,
        cursor: terminal_paint_cursor(snapshot, default_fg, default_bg),
        dirty_rows: snapshot.damage.rows.len(),
        dirty_cells: snapshot.damage.cells as usize,
        rebuilt_rows,
        reused_rows,
        repaint_backgrounds,
        rebuilt_row_flags: dirty_row_flags,
    }
}

fn terminal_row_background_submission_count(row: &TerminalPaintRowInput) -> usize {
    row.background_runs.len() + 1
}

fn terminal_prefers_full_canvas(input: &TerminalGridPaintInput) -> bool {
    // Keep one stable GPUI tree for the terminal. Switching between row views and
    // a monolithic canvas during dense TUI redraws caused stale pixels to be
    // composited into unrelated rows.
    let _ = input;
    false
}

fn terminal_grid_interaction_state(
    session_id: &str,
    render_cache: &mut TerminalRenderCache,
) -> Rc<RefCell<TerminalGridInteractionState>> {
    render_cache
        .sessions
        .get(session_id)
        .expect("terminal paint input initializes the session render cache")
        .interaction
        .clone()
}

fn clear_terminal_row_views(session_id: &str, render_cache: &mut TerminalRenderCache) {
    if let Some(cache) = render_cache.sessions.get_mut(session_id) {
        cache.row_views.fill_with(|| None);
    }
}

fn render_terminal_full_canvas(
    input: TerminalGridPaintInput,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    build_micros: u64,
    width: f32,
    height: f32,
) -> gpui::Div {
    div().w(px(width)).h(px(height)).overflow_hidden().child(
        canvas(
            move |_bounds, window, _cx| {
                let mut glyph_cache = terminal_glyph_cache.borrow_mut();
                let mut glyph_cache_hits = 0usize;
                let mut glyph_cache_misses = 0usize;
                let shaped_glyph_cells = shape_terminal_glyph_cells(
                    &input.glyph_cells,
                    &mut glyph_cache,
                    &mut glyph_cache_hits,
                    &mut glyph_cache_misses,
                    window,
                );
                let cursor_glyph_cells = input
                    .cursor
                    .as_ref()
                    .and_then(|cursor| cursor.glyph_cell.clone())
                    .into_iter()
                    .collect::<Vec<_>>();
                let shaped_cursor_glyph_cells = shape_terminal_glyph_cells(
                    &cursor_glyph_cells,
                    &mut glyph_cache,
                    &mut glyph_cache_hits,
                    &mut glyph_cache_misses,
                    window,
                );
                TerminalFullPaintSurface {
                    input,
                    shaped_glyph_cells,
                    shaped_cursor_glyph_cells,
                    glyph_cache_hits,
                    glyph_cache_misses,
                }
            },
            move |bounds, surface, window, cx| {
                let sample = terminal_full_render_profile_sample(&surface, build_micros);
                let paint_started_at = Instant::now();
                paint_terminal_full_surface(bounds, surface, window, cx);
                let mut sample = sample;
                sample.paint_micros = duration_micros(paint_started_at.elapsed());
                record_terminal_render_profile(sample);
            },
        )
        .w(px(width))
        .h(px(height))
        .overflow_hidden(),
    )
}

fn render_terminal_cursor_overlay(
    cursor: TerminalPaintCursor,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
) -> impl IntoElement {
    let left = cursor.col_index as f32 * TERMINAL_CELL_WIDTH;
    let top = cursor.row_index as f32 * TERMINAL_CELL_HEIGHT;
    let width = TERMINAL_CELL_WIDTH * f32::from(cursor.cell_width.max(1));
    canvas(
        move |_bounds, window, _cx| {
            let mut cache = terminal_glyph_cache.borrow_mut();
            let mut glyph_cache_hits = 0usize;
            let mut glyph_cache_misses = 0usize;
            let glyph_cells = cursor.glyph_cell.clone().into_iter().collect::<Vec<_>>();
            let shaped_glyph_cells = shape_terminal_glyph_cells(
                &glyph_cells,
                &mut cache,
                &mut glyph_cache_hits,
                &mut glyph_cache_misses,
                window,
            );
            TerminalCursorPaintSurface {
                cursor,
                shaped_glyph_cells,
            }
        },
        move |bounds, surface, window, cx| {
            paint_terminal_cursor_surface(bounds, &surface, window, cx);
        },
    )
    .absolute()
    .top(px(top))
    .left(px(left))
    .w(px(width))
    .h(px(TERMINAL_CELL_HEIGHT))
    .overflow_hidden()
}

fn render_terminal_selection_layer(
    live_key: String,
    selection: Option<TerminalSelection>,
    cols: u16,
    rows: u16,
    interaction: Rc<RefCell<TerminalGridInteractionState>>,
    cx: &mut Context<OcttyApp>,
) -> impl IntoElement {
    let width = TERMINAL_CELL_WIDTH * cols as f32;
    let height = TERMINAL_CELL_HEIGHT * rows as f32;
    let mouse_down_key = live_key.clone();
    let mouse_move_key = live_key.clone();
    let mouse_up_key = live_key.clone();
    let middle_click_key = live_key;
    let mouse_down_interaction = interaction.clone();
    let mouse_move_interaction = interaction.clone();
    let mouse_up_interaction = interaction.clone();

    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                let Some(point) = terminal_grid_point_from_mouse_position(
                    event.position,
                    &mouse_down_interaction.borrow(),
                    cols,
                    rows,
                ) else {
                    return;
                };
                this.start_terminal_selection(
                    &mouse_down_key,
                    point,
                    terminal_selection_mode_from_modifiers(event.modifiers),
                    cx,
                );
            }),
        )
        .on_mouse_move(
            cx.listener(move |this, event: &MouseMoveEvent, _window, cx| {
                if !event.dragging() {
                    return;
                }
                let Some(point) = terminal_grid_point_from_mouse_position(
                    event.position,
                    &mouse_move_interaction.borrow(),
                    cols,
                    rows,
                ) else {
                    return;
                };
                this.update_terminal_selection(
                    &mouse_move_key,
                    point,
                    terminal_selection_mode_from_modifiers(event.modifiers),
                    cx,
                );
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseUpEvent, _window, cx| {
                let Some(point) = terminal_grid_point_from_mouse_position(
                    event.position,
                    &mouse_up_interaction.borrow(),
                    cols,
                    rows,
                ) else {
                    this.finish_terminal_selection(&mouse_up_key, cx);
                    return;
                };
                this.update_terminal_selection(
                    &mouse_up_key,
                    point,
                    terminal_selection_mode_from_modifiers(event.modifiers),
                    cx,
                );
                this.finish_terminal_selection(&mouse_up_key, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                this.paste_terminal_primary_selection(&middle_click_key, cx);
            }),
        )
        .child(
            canvas(
                move |bounds, _window, _cx| {
                    interaction.borrow_mut().bounds = Some(bounds);
                    TerminalSelectionPaintSurface {
                        runs: selection
                            .as_ref()
                            .map(|selection| terminal_selection_runs(selection, cols, rows))
                            .unwrap_or_default(),
                    }
                },
                move |bounds, surface, window, _cx| {
                    paint_terminal_selection_surface(bounds, &surface, window);
                },
            )
            .w(px(width))
            .h(px(height)),
        )
}
