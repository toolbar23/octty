impl Render for OcttyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_live_terminals_for_active_snapshot(cx);
        self.terminal_window_active = window.is_window_active();
        self.schedule_terminal_snapshot_notifications(cx);
        self.schedule_pane_activity_reconciliation(cx);
        let taskspace_height =
            taskspace_height_for_viewport(f32::from(window.viewport_size().height));
        let taskspace_width = taskspace_width_for_viewport(f32::from(window.viewport_size().width));
        for (workspace_id, pane_id, cols, rows) in
            terminal_resize_requests(self.active_snapshot.as_ref(), taskspace_height)
        {
            self.resize_live_terminal(&workspace_id, &pane_id, cols, rows);
        }

        let shortcut_labels = workspace_shortcut_targets(&self.workspaces)
            .into_iter()
            .map(|target| (target.workspace_id, target.label))
            .collect::<HashMap<_, _>>();
        let workspace_list = render_workspace_sidebar(
            &self.project_roots,
            &self.workspaces,
            self.active_workspace_index,
            &shortcut_labels,
            &self.pane_activity,
            cx,
        );

        let taskspace = render_taskspace(
            self.active_snapshot.as_ref(),
            &self.live_terminals,
            &self.pane_activity,
            self.terminal_glyph_cache.clone(),
            self.terminal_render_cache.clone(),
            taskspace_width,
            cx,
        );
        let sidebar_menu = self.sidebar_menu.clone();
        let sidebar_rename_dialog = self
            .sidebar_rename_dialog
            .as_ref()
            .map(|dialog| (dialog.title.clone(), dialog.input.clone()));
        let toasts = self
            .toasts
            .iter()
            .map(|toast| (toast.id, toast.message.clone()))
            .collect::<Vec<_>>();
        let outside_menu_width =
            (window.viewport_size().width - px(WORKSPACE_SIDEBAR_WIDTH)).max(px(0.0));

        div()
            .id("octty-rs-root")
            .key_context("OcttyApp")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_workspace))
            .on_action(cx.listener(Self::navigate_workspace))
            .on_action(cx.listener(Self::add_project_root))
            .on_action(cx.listener(Self::add_shell_pane))
            .on_action(cx.listener(Self::add_diff_pane))
            .on_action(cx.listener(Self::add_note_pane))
            .on_action(cx.listener(Self::paste_terminal_clipboard))
            .on_action(cx.listener(Self::copy_terminal_selection))
            .on_action(cx.listener(Self::cut_terminal_selection))
            .on_action(cx.listener(Self::navigate_pane))
            .on_action(cx.listener(Self::close_active_pane))
            .on_action(cx.listener(Self::resize_focused_column))
            .on_action(cx.listener(Self::create_workspace_for_root))
            .on_action(cx.listener(Self::rename_project_root))
            .on_action(cx.listener(Self::remove_project_root))
            .on_action(cx.listener(Self::rename_workspace))
            .on_action(cx.listener(Self::forget_workspace))
            .on_action(cx.listener(Self::delete_and_forget_workspace))
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .size_full()
            .bg(rgb(0x171717))
            .text_color(rgb(0xf2f2f2))
            .child(
                div()
                    .w(px(WORKSPACE_SIDEBAR_WIDTH))
                    .h_full()
                    .border_r_1()
                    .border_color(rgb(0x4d545f))
                    .bg(rgb(0x323640))
                    .text_color(rgb(0xd7dce4))
                    .flex()
                    .flex_col()
                    .child(div().flex_1().overflow_y_scrollbar().child(workspace_list))
                    .child(render_sidebar_footer(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .p_6()
                    .child(taskspace),
            )
            .when(!toasts.is_empty(), |this| {
                this.child(deferred(
                    anchored()
                        .anchor(Corner::TopRight)
                        .position(point(px(16.0), px(16.0)))
                        .child(
                            div().w(px(420.0)).flex().flex_col().gap_2().children(
                                toasts
                                    .into_iter()
                                    .map(|(id, message)| render_error_toast(id, message, cx)),
                            ),
                        ),
                ))
            })
            .when_some(sidebar_rename_dialog, |this, (title, input)| {
                this.child(deferred(
                    anchored()
                        .anchor(Corner::TopLeft)
                        .position(point(px(0.0), px(0.0)))
                        .child(
                            div()
                                .w(window.viewport_size().width)
                                .h(window.viewport_size().height)
                                .occlude()
                                .bg(rgba(0x00000033))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _window, cx| {
                                        this.cancel_sidebar_rename_dialog(cx);
                                    }),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(WORKSPACE_SIDEBAR_WIDTH + 24.0))
                                        .top(px(64.0))
                                        .w(px(360.0))
                                        .p_3()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(rgb(0x4d545f))
                                        .bg(rgb(0x23272f))
                                        .shadow_lg()
                                        .text_color(rgb(0xd7dce4))
                                        .occlude()
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .child(
                                            div()
                                                .mb_3()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(title),
                                        )
                                        .child(Input::new(&input).appearance(false).bordered(true))
                                        .child(
                                            div()
                                                .mt_3()
                                                .flex()
                                                .justify_end()
                                                .gap_2()
                                                .child(
                                                    rename_dialog_button("Cancel").on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _window, cx| {
                                                            this.cancel_sidebar_rename_dialog(cx);
                                                        }),
                                                    ),
                                                )
                                                .child(
                                                    rename_dialog_primary_button("Rename")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(|this, _, _window, cx| {
                                                                this.confirm_sidebar_rename_dialog(
                                                                    cx,
                                                                );
                                                            }),
                                                        ),
                                                ),
                                        ),
                                ),
                        ),
                ))
            })
            .when_some(sidebar_menu, |this, overlay| {
                this.child(deferred(
                    anchored()
                        .anchor(Corner::TopLeft)
                        .position(point(px(0.0), px(0.0)))
                        .snap_to_window_with_margin(px(8.0))
                        .child(
                            div()
                                .w(window.viewport_size().width)
                                .h(window.viewport_size().height)
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(WORKSPACE_SIDEBAR_WIDTH))
                                        .top_0()
                                        .w(outside_menu_width)
                                        .h(window.viewport_size().height)
                                        .occlude()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _window, cx| {
                                                this.dismiss_sidebar_menu(cx);
                                            }),
                                        ),
                                )
                                .child(
                                    anchored()
                                        .anchor(Corner::TopLeft)
                                        .position(overlay.position)
                                        .snap_to_window_with_margin(px(8.0))
                                        .child(div().occlude().child(overlay.menu)),
                                ),
                        ),
                ))
            })
    }
}
