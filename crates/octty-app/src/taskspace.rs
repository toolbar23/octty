fn render_taskspace(
    snapshot: Option<&WorkspaceSnapshot>,
    live_terminals: &HashMap<String, LiveTerminalPane>,
    pane_activity: &HashMap<(String, String), PaneActivity>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    viewport_width: f32,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let taskspace = div().mt_4().flex_1().h_full().overflow_hidden();
    let Some(snapshot) = snapshot else {
        return taskspace.flex().child(
            div()
                .text_color(rgb(0xa0a0a0))
                .child("Open a workspace to start."),
        );
    };
    if snapshot.panes.is_empty() {
        return taskspace
            .flex()
            .child(div().text_color(rgb(0xa0a0a0)).child("No panes are open."));
    }

    let viewport_offset = taskspace_viewport_offset(snapshot, viewport_width);
    let mut panel_strip = div()
        .flex()
        .gap_3()
        .h_full()
        .ml(px(-viewport_offset))
        .flex_none();

    for column_id in &snapshot.center_column_ids {
        let Some(column) = snapshot.columns.get(column_id) else {
            continue;
        };
        let mut column_el = div()
            .flex()
            .flex_col()
            .gap_3()
            .h_full()
            .overflow_hidden()
            .flex_none()
            .w(px(column.width_px as f32));
        for pane_id in &column.pane_ids {
            if let Some(pane) = snapshot.panes.get(pane_id) {
                let active = snapshot.active_pane_id.as_deref() == Some(pane.id.as_str());
                let terminal_live =
                    live_terminals.get(&live_terminal_key(&snapshot.workspace_id, &pane.id));
                column_el = column_el.child(render_pane(
                    &snapshot.workspace_id,
                    pane,
                    active,
                    pane_activity_state(&snapshot.workspace_id, &pane.id, pane_activity),
                    terminal_live,
                    terminal_glyph_cache.clone(),
                    terminal_render_cache.clone(),
                    cx,
                ));
            }
        }
        panel_strip = panel_strip.child(column_el);
    }
    taskspace.child(panel_strip)
}

fn render_pane(
    workspace_id: &str,
    pane: &PaneState,
    active: bool,
    activity_state: ActivityState,
    terminal_live: Option<&LiveTerminalPane>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let pane_id = pane.id.clone();
    let scroll_workspace_id = workspace_id.to_owned();
    let scroll_pane_id = pane.id.clone();
    let mut pane_el = div()
        .flex()
        .flex_col()
        .flex_1()
        .overflow_hidden()
        .border_1()
        .border_color(pane_border_color(active, activity_state))
        .rounded_md()
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.select_pane(&pane_id, window, cx);
            }),
        )
        .on_scroll_wheel(cx.listener(move |this, event, _window, cx| {
            this.scroll_live_terminal(&scroll_workspace_id, &scroll_pane_id, event, cx);
        }));

    if !matches!(pane.payload, PanePayload::Terminal(_)) {
        pane_el = pane_el.child(
            div()
                .p_2()
                .border_b_1()
                .border_color(rgb(0x333333))
                .text_sm()
                .child(pane.title.clone()),
        );
    }

    pane_el.child(render_pane_body(
        workspace_id,
        &pane.id,
        pane,
        active,
        terminal_live,
        terminal_glyph_cache,
        terminal_render_cache,
        cx,
    ))
}

fn render_pane_body(
    workspace_id: &str,
    pane_id: &str,
    pane: &PaneState,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    match &pane.payload {
        PanePayload::Terminal(payload) => render_terminal_surface(
            workspace_id,
            pane_id,
            payload,
            active,
            terminal_live,
            terminal_glyph_cache,
            terminal_render_cache,
            cx,
        ),
        _ => div()
            .flex_1()
            .overflow_hidden()
            .p_3()
            .text_sm()
            .text_color(rgb(0xb8b8b8))
            .child(pane_body_label(pane)),
    }
}

fn render_terminal_surface(
    workspace_id: &str,
    pane_id: &str,
    payload: &TerminalPanePayload,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let terminal_snapshot = terminal_live.and_then(|live| live.latest.as_ref());
    let Some(snapshot) = terminal_snapshot else {
        return div()
            .flex_1()
            .overflow_hidden()
            .p_3()
            .bg(rgb(0x080a0d))
            .font(terminal_font())
            .text_size(px(TERMINAL_FONT_SIZE))
            .line_height(px(TERMINAL_CELL_HEIGHT))
            .text_color(rgb(0xc8d0d8))
            .child(terminal_screen_excerpt(&payload.restored_buffer));
    };

    let default_fg = terminal_rgb_to_rgba(snapshot.default_fg);
    let default_bg = terminal_rgb_to_rgba(snapshot.default_bg);
    let debug_timer_label = terminal_performance_data_enabled()
        .then(|| terminal_live.and_then(|live| live.latency.summary_label()))
        .flatten();
    let selection = terminal_live.and_then(|live| live.selection.as_ref());
    let live_key = live_terminal_key(workspace_id, pane_id);
    let mut surface = div()
        .flex_1()
        .overflow_hidden()
        .p_2()
        .bg(default_bg)
        .font(terminal_font())
        .text_size(px(TERMINAL_FONT_SIZE))
        .line_height(px(TERMINAL_CELL_HEIGHT));

    if let Some(label) = debug_timer_label {
        surface = surface.child(
            div()
                .text_size(px(TERMINAL_DEBUG_TIMER_FONT_SIZE))
                .line_height(px(TERMINAL_DEBUG_TIMER_LINE_HEIGHT))
                .mb(px(TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM))
                .text_color(rgb(if active { 0x6fae74 } else { 0x4f5d68 }))
                .truncate()
                .child(label),
        );
    }

    surface.child(render_terminal_grid(
        live_key,
        snapshot,
        selection,
        default_fg,
        default_bg,
        terminal_glyph_cache,
        terminal_render_cache,
        cx,
    ))
}

fn pane_body_label(pane: &PaneState) -> String {
    match &pane.payload {
        PanePayload::Terminal(payload) => {
            let screen = terminal_screen_excerpt(&payload.restored_buffer);
            if screen.is_empty() {
                String::new()
            } else {
                screen
            }
        }
        PanePayload::Note(payload) => format!(
            "Note placeholder · {}",
            payload.note_path.as_deref().unwrap_or("no note selected")
        ),
        PanePayload::Diff(_) => "Diff placeholder · JJ diff will render here.".to_owned(),
        PanePayload::Browser(payload) => format!("Browser deferred · {}", payload.url),
    }
}

fn terminal_screen_excerpt(screen: &str) -> String {
    let lines: Vec<_> = screen
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(8);
    lines[start..].join("\n")
}
