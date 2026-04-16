use super::*;
use crate::app_live_terminals::terminal_page_scroll_direction;
use crate::app_panes::{SidebarRenameDialogKeyAction, sidebar_rename_dialog_key_action};
use crate::cli::{TerminalReplayEventsStep, parse_terminal_replay_events};

fn test_project_root(id: &str, display_name: &str) -> ProjectRootRecord {
    ProjectRootRecord {
        id: id.to_owned(),
        root_path: format!("/repos/{display_name}"),
        display_name: display_name.to_owned(),
        created_at: 0,
        updated_at: 0,
    }
}

fn test_workspace(id: &str, root_id: &str, workspace_name: &str) -> WorkspaceSummary {
    WorkspaceSummary {
        id: id.to_owned(),
        root_id: root_id.to_owned(),
        root_path: format!("/repos/{root_id}"),
        project_display_name: root_id.to_owned(),
        workspace_name: workspace_name.to_owned(),
        display_name: workspace_name.to_owned(),
        workspace_path: format!("/repos/{root_id}/{workspace_name}"),
        status: Default::default(),
        created_at: 0,
        updated_at: 0,
        last_opened_at: 0,
    }
}

fn test_terminal_scroll(rows: u16) -> TerminalScrollSnapshot {
    TerminalScrollSnapshot {
        total: u64::from(rows),
        offset: 0,
        len: u64::from(rows),
    }
}

fn test_key_event(shortcut: &str) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: gpui::Keystroke::parse(shortcut).expect("parse shortcut"),
        is_held: false,
    }
}

#[test]
fn workspace_activity_marker_combines_attention_and_recent_activity() {
    assert_eq!(
        workspace_activity_marker(WorkspaceActivityIndicator {
            activity_state: ActivityState::Active,
            needs_attention: true,
        }),
        WorkspaceActivityMarker::AttentionActive
    );
    assert_eq!(
        workspace_activity_marker(WorkspaceActivityIndicator {
            activity_state: ActivityState::IdleSeen,
            needs_attention: true,
        }),
        WorkspaceActivityMarker::AttentionIdle
    );
    assert_eq!(
        workspace_activity_marker(WorkspaceActivityIndicator {
            activity_state: ActivityState::Active,
            needs_attention: false,
        }),
        WorkspaceActivityMarker::Active
    );
    assert_eq!(
        workspace_activity_marker(WorkspaceActivityIndicator {
            activity_state: ActivityState::IdleUnseen,
            needs_attention: false,
        }),
        WorkspaceActivityMarker::Idle
    );
}

#[test]
fn workspace_activity_marker_stops_spinning_when_attention_closes_activity() {
    let workspace = test_workspace("workspace-1", "root-1", "main");
    let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);
    activity.record_activity(now_ms(), None, None);
    activity.record_attention(now_ms());
    let pane_activity = [((workspace.id.clone(), "pane-1".to_owned()), activity)]
        .into_iter()
        .collect();

    assert_eq!(
        workspace_activity_marker(workspace_activity_indicator(&workspace, &pane_activity)),
        WorkspaceActivityMarker::AttentionIdle
    );
}

#[test]
fn workspace_activity_marker_stays_idle_after_attention_is_seen() {
    let workspace = test_workspace("workspace-1", "root-1", "main");
    let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);
    let now = now_ms();
    activity.record_activity(now, None, None);
    activity.record_attention(now);
    activity.record_seen(now);
    let pane_activity = [((workspace.id.clone(), "pane-1".to_owned()), activity)]
        .into_iter()
        .collect();

    assert_eq!(
        workspace_activity_marker(workspace_activity_indicator(&workspace, &pane_activity)),
        WorkspaceActivityMarker::Idle
    );
}

#[test]
fn pane_border_marker_prefers_attention_until_seen() {
    assert_eq!(
        pane_border_marker(
            true,
            PaneActivityIndicator {
                activity_state: ActivityState::Active,
                needs_attention: true,
                show_attention: true,
            },
        ),
        PaneBorderMarker::Attention
    );
    assert_eq!(
        pane_border_marker(
            false,
            PaneActivityIndicator {
                activity_state: ActivityState::IdleUnseen,
                needs_attention: true,
                show_attention: true,
            },
        ),
        PaneBorderMarker::Attention
    );
    assert_eq!(
        pane_border_marker(
            true,
            PaneActivityIndicator {
                activity_state: ActivityState::Active,
                needs_attention: false,
                show_attention: false,
            },
        ),
        PaneBorderMarker::Focused
    );
    assert_eq!(
        pane_border_marker(
            false,
            PaneActivityIndicator {
                activity_state: ActivityState::IdleUnseen,
                needs_attention: false,
                show_attention: false,
            },
        ),
        PaneBorderMarker::Unfocused
    );
}

#[test]
fn pane_border_marker_keeps_attention_visible_after_seen() {
    let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);
    activity.record_attention(2_000);
    activity.record_seen(2_100);

    assert!(pane_attention_visible(&activity, 2_500));
    assert_eq!(
        pane_attention_clear_remaining_ms(&activity, 2_500),
        Some(3_600)
    );
    assert_eq!(
        pane_border_marker(
            true,
            PaneActivityIndicator {
                activity_state: activity.state_at(2_500, PANE_ACTIVITY_ACTIVE_WINDOW_MS),
                needs_attention: activity.needs_attention,
                show_attention: pane_attention_visible(&activity, 2_500),
            },
        ),
        PaneBorderMarker::Attention
    );
    assert!(pane_attention_visible(&activity, 5_500));
    assert!(!pane_attention_visible(&activity, 6_100));

    activity.record_seen(10_000);
    assert!(!pane_attention_visible(&activity, 10_000));
}

#[test]
fn sidebar_workspace_groups_follow_repo_order_and_keep_orphans() {
    let project_roots = vec![
        test_project_root("root-b", "bravo"),
        test_project_root("root-a", "alpha"),
    ];
    let workspaces = vec![
        test_workspace("workspace-1", "root-a", "one"),
        test_workspace("workspace-2", "missing-root", "two"),
        test_workspace("workspace-3", "root-b", "three"),
    ];

    let groups = sidebar_workspace_groups(&project_roots, &workspaces);

    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups[0].root.as_ref().map(|root| root.id.as_str()),
        Some("root-b")
    );
    assert_eq!(groups[0].workspace_indices, vec![2]);
    assert_eq!(
        groups[1].root.as_ref().map(|root| root.id.as_str()),
        Some("root-a")
    );
    assert_eq!(groups[1].workspace_indices, vec![0]);
    assert_eq!(groups[2].root, None);
    assert_eq!(groups[2].workspace_indices, vec![1]);
}

#[test]
fn printable_keys_become_terminal_text() {
    let input =
        live_terminal_input_from_key_parts("a", Some("a"), false, false, false, false, false)
            .expect("printable input");
    assert_eq!(input.key, LiveTerminalKey::Character('a'));
    assert_eq!(input.text.as_deref(), Some("a"));

    let shifted =
        live_terminal_input_from_key_parts("a", Some("A"), false, false, true, false, false)
            .expect("shifted input");
    assert_eq!(shifted.key, LiveTerminalKey::Character('a'));
    assert_eq!(shifted.text.as_deref(), Some("A"));
    assert!(shifted.modifiers.shift);
}

#[test]
fn named_keys_become_terminal_keys() {
    let enter =
        live_terminal_input_from_key_parts("enter", None, false, false, false, false, false)
            .expect("enter input");
    assert_eq!(enter.key, LiveTerminalKey::Enter);

    let return_key =
        live_terminal_input_from_key_parts("return", None, false, false, false, false, false)
            .expect("return input");
    assert_eq!(return_key.key, LiveTerminalKey::Enter);

    let backspace =
        live_terminal_input_from_key_parts("backspace", None, false, false, false, false, false)
            .expect("backspace input");
    assert_eq!(backspace.key, LiveTerminalKey::Backspace);

    let page_up =
        live_terminal_input_from_key_parts("pageup", None, false, false, false, false, false)
            .expect("page-up input");
    assert_eq!(page_up.key, LiveTerminalKey::PageUp);

    let page_down =
        live_terminal_input_from_key_parts("pagedown", None, false, false, false, false, false)
            .expect("page-down input");
    assert_eq!(page_down.key, LiveTerminalKey::PageDown);
}

#[test]
fn page_keys_map_to_viewport_scroll_directions() {
    let page_up =
        live_terminal_input_from_key_parts("pageup", None, false, false, false, false, false)
            .expect("page-up input");
    assert_eq!(terminal_page_scroll_direction(&page_up), Some(-1));

    let page_down =
        live_terminal_input_from_key_parts("pagedown", None, false, false, false, false, false)
            .expect("page-down input");
    assert_eq!(terminal_page_scroll_direction(&page_down), Some(1));

    let modified =
        live_terminal_input_from_key_parts("pageup", None, true, false, false, false, false)
            .expect("modified page-up input");
    assert_eq!(terminal_page_scroll_direction(&modified), None);
}

#[test]
fn shift_return_becomes_control_j_for_terminal() {
    let input = live_terminal_input_from_key_parts("enter", None, false, false, true, false, false)
        .expect("shift-enter input");

    assert_eq!(input.key, LiveTerminalKey::Character('j'));
    assert_eq!(input.text, None);
    assert!(input.modifiers.control);
    assert!(!input.modifiers.shift);
    assert_eq!(input.unshifted, 'j');

    let return_key =
        live_terminal_input_from_key_parts("return", None, false, false, true, false, false)
            .expect("shift-return input");
    assert_eq!(return_key, input);

    let modified =
        live_terminal_input_from_key_parts("enter", None, true, false, true, false, false)
            .expect("ctrl-shift-enter input");
    assert_eq!(modified.key, LiveTerminalKey::Enter);
    assert!(modified.modifiers.control);
    assert!(modified.modifiers.shift);
}

#[test]
fn tab_keys_become_terminal_tab_keys() {
    let tab = live_terminal_input_from_key_parts("tab", None, false, false, false, false, false)
        .expect("tab input");
    assert_eq!(tab, terminal_tab_input(false));

    let tab_with_key_char =
        live_terminal_input_from_key_parts("tab", Some("\t"), false, false, false, false, false)
            .expect("tab input with key char");
    assert_eq!(tab_with_key_char, terminal_tab_input(false));

    let shift_tab =
        live_terminal_input_from_key_parts("tab", None, false, false, true, false, false)
            .expect("shift-tab input");
    assert_eq!(shift_tab, terminal_tab_input(true));
}

#[test]
fn retach_fallback_maps_tab_and_control_j() {
    assert_eq!(
        retach_key_for_live_key(&terminal_tab_input(false)).as_deref(),
        Some("Tab")
    );
    assert_eq!(
        retach_key_for_live_key(&terminal_control_j_input()).as_deref(),
        Some("C-j")
    );
}

#[test]
fn space_key_forwards_printable_space_text() {
    let input =
        live_terminal_input_from_key_parts("space", None, false, false, false, false, false)
            .expect("space input");

    assert_eq!(input.key, LiveTerminalKey::Character(' '));
    assert_eq!(input.text.as_deref(), Some(" "));
    assert_eq!(input.unshifted, ' ');
}

#[test]
fn printable_key_char_takes_precedence_over_named_key() {
    let input =
        live_terminal_input_from_key_parts("space", Some(" "), false, false, false, false, false)
            .expect("printable space input");

    assert_eq!(input.key, LiveTerminalKey::Character(' '));
    assert_eq!(input.text.as_deref(), Some(" "));
    assert_eq!(input.unshifted, ' ');
}

#[test]
fn unmodified_single_character_key_synthesizes_text_when_key_char_is_missing() {
    let input = live_terminal_input_from_key_parts("a", None, false, false, false, false, false)
        .expect("synthesized printable input");

    assert_eq!(input.key, LiveTerminalKey::Character('a'));
    assert_eq!(input.text.as_deref(), Some("a"));
    assert_eq!(input.unshifted, 'a');
}

#[test]
fn control_space_keeps_control_modifier_without_text() {
    let input = live_terminal_input_from_key_parts("space", None, true, false, false, false, false)
        .expect("control-space input");

    assert_eq!(input.key, LiveTerminalKey::Space);
    assert_eq!(input.text, None);
    assert!(input.modifiers.control);
    assert_eq!(input.unshifted, ' ');
}

#[test]
fn named_keys_do_not_forward_key_char_as_text() {
    let escape = live_terminal_input_from_key_parts(
        "escape",
        Some("\x1b"),
        false,
        false,
        false,
        false,
        false,
    )
    .expect("escape input");
    assert_eq!(escape.key, LiveTerminalKey::Escape);
    assert_eq!(escape.text, None);

    let up =
        live_terminal_input_from_key_parts("up", Some("\x1b[A"), false, false, false, false, false)
            .expect("up input");
    assert_eq!(up.key, LiveTerminalKey::ArrowUp);
    assert_eq!(up.text, None);
}

#[test]
fn control_characters_are_not_forwarded_as_text() {
    let escape = live_terminal_input_from_key_parts(
        "escape",
        Some("\x1b"),
        false,
        false,
        false,
        false,
        false,
    )
    .expect("escape input");
    assert_eq!(escape.text, None);
}

#[test]
fn terminal_input_preserves_workspace_shortcuts() {
    assert_eq!(
        live_terminal_input_from_key_parts("1", Some("!"), true, false, true, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("!", Some("!"), true, false, true, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("up", None, true, false, true, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("down", None, true, false, true, false, false),
        None
    );
}

#[test]
fn workspace_shortcut_index_accepts_shifted_digit_key_tokens() {
    assert_eq!(
        workspace_shortcut_index_from_key_parts("1", Some("!"), true, false, true, false, false),
        Some(0)
    );
    assert_eq!(
        workspace_shortcut_index_from_key_parts("0", Some(")"), true, false, true, false, false),
        Some(9)
    );
    assert_eq!(
        workspace_shortcut_index_from_key_parts("!", Some("!"), true, false, true, false, false),
        Some(0)
    );
    assert_eq!(
        workspace_shortcut_index_from_key_parts("1", Some("!"), true, false, false, false, false),
        None
    );
    assert_eq!(
        workspace_shortcut_index_from_key_parts("1", Some("!"), true, true, true, false, false),
        None
    );
}

#[test]
fn terminal_input_preserves_paste_shortcut() {
    for key in ["c", "x", "v"] {
        let key_char = key.to_ascii_uppercase();
        assert_eq!(
            live_terminal_input_from_key_parts(
                key,
                Some(key_char.as_str()),
                true,
                false,
                true,
                false,
                false
            ),
            None
        );
    }
    assert!(
        live_terminal_input_from_key_parts("p", Some("P"), true, false, true, false, false)
            .is_some()
    );
    for key in ["c", "x", "v", "p"] {
        assert_eq!(
            live_terminal_input_from_key_parts(key, None, false, false, false, true, false),
            None
        );
    }
    assert_eq!(
        live_terminal_input_from_key_parts("KeyC", Some("c"), false, false, false, true, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("KeyX", Some("x"), false, false, false, true, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("KeyV", Some("v"), false, false, false, true, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("KeyP", Some("p"), false, false, false, true, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("insert", None, true, false, false, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("insert", None, false, false, true, false, false),
        None
    );
    assert!(
        live_terminal_input_from_key_parts("x", Some("x"), true, false, false, false, false)
            .is_some()
    );
}

#[test]
fn platform_clipboard_shortcuts_are_app_actions() {
    assert_eq!(
        clipboard_shortcut_action_from_key_event(&test_key_event("super-c")),
        Some(ClipboardShortcutAction::Copy)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_event(&test_key_event("super-x")),
        Some(ClipboardShortcutAction::Cut)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_event(&test_key_event("super-v")),
        Some(ClipboardShortcutAction::Paste)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_event(&test_key_event("super-p")),
        Some(ClipboardShortcutAction::Paste)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts(
            "KeyC",
            Some("c"),
            false,
            false,
            false,
            true,
            false
        ),
        Some(ClipboardShortcutAction::Copy)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts(
            "KeyV",
            Some("v"),
            false,
            false,
            false,
            true,
            false
        ),
        Some(ClipboardShortcutAction::Paste)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts(
            "KeyX",
            Some("x"),
            false,
            false,
            false,
            true,
            false
        ),
        Some(ClipboardShortcutAction::Cut)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts(
            "KeyP",
            Some("p"),
            false,
            false,
            false,
            true,
            false
        ),
        Some(ClipboardShortcutAction::Paste)
    );
}

#[test]
fn rewritten_clipboard_shortcuts_are_app_actions() {
    assert_eq!(
        clipboard_shortcut_action_from_key_parts("insert", None, true, false, false, false, false),
        Some(ClipboardShortcutAction::Copy)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts("insert", None, false, false, true, false, false),
        Some(ClipboardShortcutAction::Paste)
    );
    assert_eq!(
        clipboard_shortcut_action_from_key_parts("x", Some("x"), true, false, false, false, false),
        None
    );
}

#[test]
fn terminal_input_preserves_pane_action_shortcuts() {
    assert_eq!(
        live_terminal_input_from_key_parts("left", None, true, false, true, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("w", Some("W"), true, false, true, false, false),
        None
    );
}

#[test]
fn terminal_input_preserves_column_resize_shortcuts() {
    assert_eq!(
        live_terminal_input_from_key_parts("left", None, true, true, false, false, false),
        None
    );
    assert_eq!(
        live_terminal_input_from_key_parts("right", None, true, true, false, false, false),
        None
    );
}

#[test]
fn control_letters_keep_control_modifier_for_encoder() {
    let input = live_terminal_input_from_key_parts("c", None, true, false, false, false, false)
        .expect("control input");
    assert_eq!(input.key, LiveTerminalKey::Character('c'));
    assert!(input.modifiers.control);
}

#[test]
fn rename_dialog_only_intercepts_commit_keys() {
    assert_eq!(
        sidebar_rename_dialog_key_action("enter"),
        Some(SidebarRenameDialogKeyAction::Confirm)
    );
    assert_eq!(
        sidebar_rename_dialog_key_action("return"),
        Some(SidebarRenameDialogKeyAction::Confirm)
    );
    assert_eq!(
        sidebar_rename_dialog_key_action("escape"),
        Some(SidebarRenameDialogKeyAction::Cancel)
    );
    assert_eq!(sidebar_rename_dialog_key_action("a"), None);
    assert_eq!(sidebar_rename_dialog_key_action("space"), None);
}

#[test]
fn default_shell_type_config_defines_initial_types() {
    let config = default_shell_type_config().expect("default shell config");
    let names = config
        .shell_types
        .iter()
        .map(|shell_type| shell_type.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["plain", "codex", "jjui"]);
    assert_eq!(config.shell_types[0].shortcut, "ctrl-shift-s");
    assert_eq!(config.shell_types[1].shortcut, "ctrl-shift-a");
    assert_eq!(config.shell_types[1].command, "codex");
    assert_eq!(
        config.shell_types[1].command_parameters,
        vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()]
    );
    assert_eq!(config.shell_types[2].shortcut, "ctrl-shift-j");
}

#[test]
fn shell_pane_state_copies_configured_metadata() {
    let config = default_shell_type_config().expect("default shell config");
    let codex = config
        .shell_types
        .iter()
        .find(|shell_type| shell_type.name == "codex")
        .expect("codex shell type");
    let pane = shell_pane_state_for_config(codex, "/tmp/workspace");
    let PanePayload::Terminal(payload) = pane.payload else {
        panic!("expected terminal payload");
    };

    assert_eq!(pane.title, "codex");
    assert_eq!(payload.kind, TerminalKind::Codex);
    assert_eq!(payload.shell_type, "codex");
    assert_eq!(payload.cwd, "/tmp/workspace");
    assert_eq!(payload.command, "codex");
    assert_eq!(payload.on_exit, TerminalExitBehavior::RestartManually);
    assert_eq!(payload.default_width_chars, 120);
}

#[test]
fn workspace_key_bindings_include_configured_shell_shortcuts() {
    let config = default_shell_type_config().expect("default shell config");
    let bindings = workspace_key_bindings(&config.shell_types);
    let codex_key = gpui::Keystroke::parse("ctrl-shift-a").expect("parse ctrl-shift-a");
    let codex_action = AddShellPane {
        shell_type: "codex".to_owned(),
    };

    let binding = bindings
        .into_iter()
        .find(|binding| binding.match_keystrokes(&[codex_key.clone()]) == Some(false))
        .expect("ctrl-shift-a binding");

    assert!(binding.action().partial_eq(&codex_action));
}

#[test]
fn configured_shell_shortcuts_match_key_events_before_terminal_input() {
    let config = default_shell_type_config().expect("default shell config");

    assert_eq!(
        shell_type_shortcut_from_key_event(&config.shell_types, &test_key_event("ctrl-shift-a")),
        Some("codex".to_owned())
    );
    assert_eq!(
        shell_type_shortcut_from_key_event(&config.shell_types, &test_key_event("ctrl-shift-j")),
        Some("jjui".to_owned())
    );
}

#[test]
fn css_font_stack_prefers_first_real_family() {
    assert_eq!(
        first_font_family("\"Iosevka Term\", monospace").as_deref(),
        Some("Iosevka Term")
    );
    assert_eq!(
        first_font_family("monospace, \"JetBrains Mono\"").as_deref(),
        Some("JetBrains Mono")
    );
}

#[test]
fn terminal_paste_normalizes_newlines_to_carriage_returns() {
    assert_eq!(
        terminal_paste_bytes("one\ntwo\r\nthree"),
        b"one\rtwo\rthree"
    );
}

#[test]
fn terminal_clipboard_paste_quotes_image_path() {
    assert_eq!(
        quote_terminal_path_for_paste("/tmp/screenshot.png"),
        "/tmp/screenshot.png"
    );
    assert_eq!(
        quote_terminal_path_for_paste("/tmp/screen shot.png"),
        "'/tmp/screen shot.png'"
    );
    assert_eq!(
        quote_terminal_path_for_paste("/tmp/it's.png"),
        "'/tmp/it'\\''s.png'"
    );
}

#[test]
fn terminal_clipboard_paste_writes_image_to_temp_file() {
    let image = Image::from_bytes(ImageFormat::Png, vec![1, 2, 3, 4]);
    let clipboard = ClipboardItem::new_image(&image);

    let paste = terminal_clipboard_paste_text(&clipboard)
        .expect("image clipboard paste")
        .expect("paste text");
    let path = PathBuf::from(paste.trim_matches('\''));

    assert!(path.is_absolute());
    assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("png"));
    assert_eq!(fs::read(&path).expect("read temp image"), vec![1, 2, 3, 4]);
    let _ = fs::remove_file(path);
}

#[test]
fn terminal_selection_runs_merge_rows() {
    let selection = TerminalSelection {
        anchor: TerminalGridPoint { row: 0, col: 2 },
        active: TerminalGridPoint { row: 2, col: 1 },
        mode: TerminalSelectionMode::default(),
    };

    assert_eq!(
        terminal_selection_runs(&selection, 5, 3),
        vec![
            TerminalSelectionRun {
                row: 0,
                start_col: 2,
                end_col: 5,
            },
            TerminalSelectionRun {
                row: 1,
                start_col: 0,
                end_col: 5,
            },
            TerminalSelectionRun {
                row: 2,
                start_col: 0,
                end_col: 2,
            },
        ]
    );

    let reversed = TerminalSelection {
        anchor: selection.active,
        active: selection.anchor,
        mode: selection.mode,
    };
    assert_eq!(
        terminal_selection_runs(&reversed, 5, 3),
        terminal_selection_runs(&selection, 5, 3)
    );
}

#[test]
fn terminal_selection_text_trims_row_padding() {
    let mut snapshot = test_terminal_snapshot("selection-text", 6, 2, vec![0, 1], true);
    write_picker_text(
        &mut snapshot.rows_data[0].cells,
        0,
        "abc   ",
        None,
        None,
        false,
        false,
    );
    write_picker_text(
        &mut snapshot.rows_data[1].cells,
        0,
        "de f  ",
        None,
        None,
        false,
        false,
    );
    let selection = TerminalSelection {
        anchor: TerminalGridPoint { row: 0, col: 1 },
        active: TerminalGridPoint { row: 1, col: 3 },
        mode: TerminalSelectionMode::default(),
    };

    assert_eq!(terminal_selection_text(&snapshot, &selection), "bc\nde f");
}

#[test]
fn terminal_selection_runs_box_columns() {
    let selection = TerminalSelection {
        anchor: TerminalGridPoint { row: 3, col: 4 },
        active: TerminalGridPoint { row: 1, col: 1 },
        mode: TerminalSelectionMode {
            rectangular: true,
            filter_indent: false,
        },
    };

    assert_eq!(
        terminal_selection_runs(&selection, 8, 5),
        vec![
            TerminalSelectionRun {
                row: 1,
                start_col: 1,
                end_col: 5,
            },
            TerminalSelectionRun {
                row: 2,
                start_col: 1,
                end_col: 5,
            },
            TerminalSelectionRun {
                row: 3,
                start_col: 1,
                end_col: 5,
            },
        ]
    );
}

#[test]
fn terminal_selection_text_box_preserves_selected_columns() {
    let mut snapshot = test_terminal_snapshot("box-selection-text", 8, 3, vec![0, 1, 2], true);
    write_picker_text(
        &mut snapshot.rows_data[0].cells,
        0,
        "  alpha",
        None,
        None,
        false,
        false,
    );
    write_picker_text(
        &mut snapshot.rows_data[1].cells,
        0,
        "  beta ",
        None,
        None,
        false,
        false,
    );
    write_picker_text(
        &mut snapshot.rows_data[2].cells,
        0,
        "  gam  ",
        None,
        None,
        false,
        false,
    );
    let selection = TerminalSelection {
        anchor: TerminalGridPoint { row: 0, col: 2 },
        active: TerminalGridPoint { row: 2, col: 5 },
        mode: TerminalSelectionMode {
            rectangular: true,
            filter_indent: false,
        },
    };

    assert_eq!(
        terminal_selection_text(&snapshot, &selection),
        "alph\nbeta\ngam "
    );
}

#[test]
fn terminal_selection_text_filter_removes_common_indent() {
    assert_eq!(
        terminal_selection_text_remove_common_indent("    one\n      two\n    three"),
        "one\n  two\nthree"
    );
}

#[test]
fn terminal_grid_point_from_local_position_clamps_to_grid() {
    assert_eq!(
        terminal_grid_point_from_local_position(point(px(0.0), px(0.0)), 10, 4),
        TerminalGridPoint { row: 0, col: 0 }
    );
    assert_eq!(
        terminal_grid_point_from_local_position(
            point(
                px(TERMINAL_CELL_WIDTH * 9.7),
                px(TERMINAL_CELL_HEIGHT * 3.9)
            ),
            10,
            4
        ),
        TerminalGridPoint { row: 3, col: 9 }
    );
    assert_eq!(
        terminal_grid_point_from_local_position(
            point(
                px(TERMINAL_CELL_WIDTH * 99.0),
                px(TERMINAL_CELL_HEIGHT * 99.0)
            ),
            10,
            4
        ),
        TerminalGridPoint { row: 3, col: 9 }
    );
}

#[test]
fn latency_summary_reports_millisecond_percentiles() {
    let samples = VecDeque::from([500, 1_500, 8_000]);
    let summary = latency_summary(&samples).expect("latency summary");
    assert!(summary.contains("p50 1.5ms"));
    assert!(summary.contains("max 8.0ms"));
}

#[test]
fn terminal_env_value_enabled_treats_zero_and_false_as_off() {
    assert!(!terminal_env_value_enabled("0"));
    assert!(!terminal_env_value_enabled("false"));
    assert!(!terminal_env_value_enabled("FALSE"));
    assert!(terminal_env_value_enabled("1"));
    assert!(terminal_env_value_enabled("true"));
}

#[test]
fn terminal_render_profiler_keeps_zero_count_samples() {
    let mut profiler = TerminalRenderProfiler::default();

    profiler.record(TerminalRenderProfileSample {
        build_micros: 10,
        rows: 1,
        cols: 3,
        dirty_rows: 0,
        dirty_cells: 0,
        rebuilt_rows: 0,
        reused_rows: 1,
        glyph_cells: 0,
        background_runs: 0,
        text_bytes: 0,
        ..TerminalRenderProfileSample::default()
    });
    profiler.record(TerminalRenderProfileSample {
        paint_micros: 0,
        rows: 1,
        cols: 3,
        painted_rows: 1,
        submitted_glyphs: 0,
        submitted_backgrounds: 1,
        ..TerminalRenderProfileSample::default()
    });

    assert_eq!(profiler.dirty_rows, VecDeque::from([0]));
    assert_eq!(profiler.dirty_cells, VecDeque::from([0]));
    assert_eq!(profiler.rebuilt_rows, VecDeque::from([0]));
    assert_eq!(profiler.glyph_cells, VecDeque::from([0]));
    assert_eq!(profiler.submitted_glyphs, VecDeque::from([0]));
    assert_eq!(profiler.paint_micros, VecDeque::from([0]));
    assert!(
        profiler
            .summary()
            .expect("profile summary")
            .contains("row paint")
    );
}

#[test]
fn terminal_notification_drain_coalesces_queued_wakeups() {
    let (tx, mut rx) = mpsc::unbounded();
    tx.unbounded_send(()).expect("first wakeup");
    tx.unbounded_send(()).expect("second wakeup");

    drain_pending_terminal_notifications(&mut rx);

    assert!(rx.try_recv().is_err());
}

#[test]
fn terminal_snapshot_coalesce_keeps_first_recent_focused_input_immediate() {
    let now = Instant::now();
    assert_eq!(
        terminal_snapshot_coalesce_interval(true, true, None, now),
        Duration::ZERO
    );
    assert_eq!(
        terminal_snapshot_coalesce_interval(true, false, None, now),
        TERMINAL_FOCUSED_FRAME_INTERVAL
    );
    assert_eq!(
        terminal_snapshot_coalesce_interval(false, true, None, now),
        TERMINAL_BACKGROUND_FRAME_INTERVAL
    );
}

#[test]
fn terminal_snapshot_coalesce_limits_recent_focused_input_to_frame_interval() {
    let now = Instant::now();
    assert_eq!(
        terminal_snapshot_coalesce_interval(true, true, Some(now - Duration::from_millis(3)), now),
        TERMINAL_FOCUSED_FRAME_INTERVAL - Duration::from_millis(3)
    );
    assert_eq!(
        terminal_snapshot_coalesce_interval(
            true,
            true,
            Some(now - TERMINAL_FOCUSED_FRAME_INTERVAL),
            now
        ),
        Duration::ZERO
    );
}

#[test]
fn terminal_snapshot_coalesce_keeps_dirty_rows_from_skipped_snapshots() {
    let first = test_terminal_snapshot("session", 4, 3, vec![0, 1], false);
    let second = test_terminal_snapshot("session", 4, 3, vec![2], false);

    let snapshot = coalesce_terminal_snapshots(vec![first, second]).expect("coalesced snapshot");

    assert_eq!(snapshot.damage.rows, vec![0, 1, 2]);
    assert!(snapshot.damage.full);
    assert_eq!(snapshot.damage.cells, 12);
    assert_eq!(snapshot.timing.dirty_rows, 3);
    assert_eq!(snapshot.timing.dirty_cells, 12);
}

#[test]
fn terminal_snapshot_coalesce_forces_full_damage_on_resize() {
    let first = test_terminal_snapshot("session", 4, 3, vec![1], false);
    let second = test_terminal_snapshot("session", 4, 2, vec![0], false);

    let snapshot = coalesce_terminal_snapshots(vec![first, second]).expect("coalesced snapshot");

    assert_eq!(snapshot.rows, 2);
    assert_eq!(snapshot.damage.rows, vec![0, 1]);
    assert!(snapshot.damage.full);
    assert_eq!(snapshot.damage.cells, 8);
}

#[test]
fn terminal_snapshot_full_damage_marks_every_row_dirty() {
    let mut snapshot = test_terminal_snapshot("session", 4, 3, vec![1], false);

    mark_terminal_snapshot_full_damage(&mut snapshot);

    assert_eq!(snapshot.damage.rows, vec![0, 1, 2]);
    assert!(snapshot.damage.full);
    assert_eq!(snapshot.damage.cells, 12);
    assert_eq!(snapshot.timing.dirty_rows, 3);
    assert_eq!(snapshot.timing.dirty_cells, 12);
}

#[test]
fn terminal_snapshot_presentation_keeps_focused_terminal_immediate() {
    let now = Instant::now();

    assert_eq!(
        terminal_snapshot_presentation_delay_for_state(true, Some(now), true, now),
        None
    );
}

#[test]
fn terminal_snapshot_presentation_rate_limits_background_terminal() {
    let now = Instant::now();

    assert_eq!(
        terminal_snapshot_presentation_delay_for_state(true, Some(now), false, now),
        Some(TERMINAL_BACKGROUND_FRAME_INTERVAL)
    );
}

#[test]
fn terminal_snapshot_presentation_releases_background_after_interval() {
    let now = Instant::now();

    assert_eq!(
        terminal_snapshot_presentation_delay_for_state(
            true,
            Some(now - TERMINAL_BACKGROUND_FRAME_INTERVAL),
            false,
            now
        ),
        None
    );
}

#[test]
fn terminal_replay_event_parser_keeps_resizes_and_output_order() {
    let events = "\
3 kind=start session=s cols=90 rows=20 output=/tmp/octty-record/session.pty
9 kind=resize cols=87 rows=52 pixel_width=696 pixel_height=936
10 kind=output offset=0 len=258 hex=1b5b
11 kind=input source=key len=1 hex=6e
12 kind=resize cols=87 rows=18 pixel_width=696 pixel_height=324
13 kind=output offset=258 len=224 hex=1b5b
";

    let plan = parse_terminal_replay_events(events).expect("parsed trace");

    assert_eq!(
        plan.output_path,
        PathBuf::from("/tmp/octty-record/session.pty")
    );
    assert_eq!(plan.initial_cols, 90);
    assert_eq!(plan.initial_rows, 20);
    assert_eq!(
        plan.steps,
        vec![
            TerminalReplayEventsStep::Resize { cols: 87, rows: 52 },
            TerminalReplayEventsStep::Output {
                offset: 0,
                len: 258
            },
            TerminalReplayEventsStep::Resize { cols: 87, rows: 18 },
            TerminalReplayEventsStep::Output {
                offset: 258,
                len: 224
            },
        ]
    );
}

#[test]
fn terminal_paint_input_shapes_only_visible_text_cells() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 3,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot::default(),
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![
                octty_term::live::TerminalCellSnapshot {
                    text: String::new(),
                    width: 1,
                    fg: None,
                    bg: None,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    underline: false,
                    inverse: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                },
                octty_term::live::TerminalCellSnapshot {
                    text: "a".to_owned(),
                    width: 1,
                    fg: None,
                    bg: None,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    underline: false,
                    inverse: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                },
                octty_term::live::TerminalCellSnapshot {
                    text: String::new(),
                    width: 1,
                    fg: None,
                    bg: None,
                    bold: false,
                    italic: false,
                    faint: false,
                    blink: false,
                    underline: false,
                    inverse: false,
                    invisible: false,
                    strikethrough: false,
                    overline: false,
                },
            ],
        }],
        plain_text: " a\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };

    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(input.glyph_cells.len(), 1);
    assert_eq!(input.glyph_cells[0].col_index, 1);
    assert_eq!(input.glyph_cells[0].text.as_ref(), "a");
    assert!(input.rows_data[0].background_runs.is_empty());
}

#[test]
fn terminal_background_runs_ignore_foreground_style_splits() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let highlighted_bg = TerminalRgb {
        r: 20,
        g: 60,
        b: 80,
    };
    let snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 4,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot::default(),
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![
                picker_cell(
                    "a",
                    Some(TerminalRgb { r: 255, g: 0, b: 0 }),
                    Some(highlighted_bg),
                    false,
                    false,
                ),
                picker_cell(
                    "b",
                    Some(TerminalRgb { r: 0, g: 255, b: 0 }),
                    Some(highlighted_bg),
                    true,
                    false,
                ),
                picker_cell(
                    "c",
                    Some(TerminalRgb { r: 0, g: 0, b: 255 }),
                    Some(highlighted_bg),
                    false,
                    true,
                ),
                picker_cell("d", None, Some(highlighted_bg), false, false),
            ],
        }],
        plain_text: "abcd\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };

    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(input.rows_data[0].background_runs.len(), 1);
    assert_eq!(input.rows_data[0].background_runs[0].start_col, 0);
    assert_eq!(input.rows_data[0].background_runs[0].cell_count, 4);
}

#[test]
fn terminal_background_runs_render_inverse_default_colors() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut inverse_cell = picker_cell("a", None, None, false, false);
    inverse_cell.inverse = true;
    let snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 2,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot::default(),
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![inverse_cell, picker_cell("b", None, None, false, false)],
        }],
        plain_text: "ab\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };

    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(input.rows_data[0].background_runs.len(), 1);
    assert_eq!(input.rows_data[0].background_runs[0].start_col, 0);
    assert_eq!(input.rows_data[0].background_runs[0].cell_count, 1);
    assert_eq!(
        input.rows_data[0].background_runs[0].color,
        terminal_rgb_to_rgba(default_fg)
    );
    assert_eq!(
        input.glyph_cells[0].color,
        Hsla::from(terminal_rgb_to_rgba(default_bg))
    );
}

#[test]
fn terminal_paint_input_rebuilds_only_dirty_rows() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 2,
        rows: 2,
        scroll: test_terminal_scroll(2),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: vec![0, 1],
            cells: 4,
        },
        rows_data: vec![
            octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell("a", None, None, false, false),
                    picker_cell("", None, None, false, false),
                ],
            },
            octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell("x", None, None, false, false),
                    picker_cell("", None, None, false, false),
                ],
            },
        ],
        plain_text: "a\nx\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };
    let mut render_cache = TerminalRenderCache::default();

    let first = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );
    assert_eq!(first.rebuilt_rows, 2);
    assert_eq!(first.reused_rows, 0);

    snapshot.damage = octty_term::live::TerminalDamageSnapshot {
        full: false,
        rows: vec![1],
        cells: 2,
    };
    snapshot.rows_data[1].cells[0].text = "b".to_owned();
    let second = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(second.rebuilt_rows, 1);
    assert_eq!(second.reused_rows, 1);
    assert_eq!(second.repaint_backgrounds, 1);
    assert!(!second.glyph_cells.iter().any(|cell| cell.row_index == 0));
    assert!(
        second
            .glyph_cells
            .iter()
            .any(|cell| cell.row_index == 1 && cell.text.as_ref() == "b")
    );

    let cache = render_cache
        .sessions
        .get(&snapshot.session_id)
        .expect("session render cache");
    let reused_row = terminal_row_view_payload(&second, cache, 0);
    assert_eq!(reused_row.glyph_cells.len(), 1);
    assert_eq!(reused_row.glyph_cells[0].text.as_ref(), "a");
}

#[test]
fn terminal_paint_input_keeps_cursor_out_of_row_cache() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut snapshot = TerminalGridSnapshot {
        session_id: "cursor-overlay-test".to_owned(),
        cols: 2,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: Some(octty_term::live::TerminalCursorSnapshot {
            col: 0,
            row: 0,
            visible: true,
        }),
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: vec![0],
            cells: 2,
        },
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![
                picker_cell("a", None, None, false, false),
                picker_cell("b", None, None, false, false),
            ],
        }],
        plain_text: "ab\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };
    let mut render_cache = TerminalRenderCache::default();

    let first = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );
    assert_eq!(first.rebuilt_rows, 1);
    assert_eq!(first.rows_data[0].background_runs.len(), 0);
    assert_eq!(
        first.glyph_cells[0].color,
        Hsla::from(terminal_rgb_to_rgba(default_fg))
    );
    assert_eq!(
        first.cursor.as_ref().map(|cursor| cursor.col_index),
        Some(0)
    );
    assert_eq!(
        first
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.glyph_cell.as_ref())
            .map(|cell| (cell.text.to_string(), cell.color)),
        Some(("a".to_owned(), Hsla::from(terminal_rgb_to_rgba(default_bg))))
    );

    snapshot.cursor = Some(octty_term::live::TerminalCursorSnapshot {
        col: 1,
        row: 0,
        visible: true,
    });
    snapshot.damage = octty_term::live::TerminalDamageSnapshot::default();
    let second = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(second.rebuilt_rows, 0);
    assert_eq!(second.reused_rows, 1);
    assert!(second.glyph_cells.is_empty());
    assert_eq!(
        second.cursor.as_ref().map(|cursor| cursor.col_index),
        Some(1)
    );
    assert_eq!(
        second
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.glyph_cell.as_ref())
            .map(|cell| cell.text.to_string()),
        Some("b".to_owned())
    );
}

#[test]
fn terminal_focus_only_render_reuses_row_cache() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut snapshot = TerminalGridSnapshot {
        session_id: "focus-overlay-test".to_owned(),
        cols: 3,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: Some(octty_term::live::TerminalCursorSnapshot {
            col: 1,
            row: 0,
            visible: true,
        }),
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: vec![0],
            cells: 3,
        },
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![
                picker_cell("a", None, None, false, false),
                picker_cell("b", None, None, false, false),
                picker_cell("c", None, None, false, false),
            ],
        }],
        plain_text: "abc\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };
    let mut render_cache = TerminalRenderCache::default();

    let _ = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    snapshot.damage = octty_term::live::TerminalDamageSnapshot::default();
    let focus_only = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(focus_only.rebuilt_rows, 0);
    assert_eq!(focus_only.reused_rows, 1);
    assert!(focus_only.glyph_cells.is_empty());
    assert_eq!(
        focus_only.cursor.as_ref().map(|cursor| cursor.col_index),
        Some(1)
    );
}

#[test]
fn terminal_paint_input_keeps_glyphs_on_original_cell_columns() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 3,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot::default(),
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![
                picker_cell("\u{65}\u{301}", None, None, false, false),
                picker_cell("", None, None, false, false),
                picker_cell("x", None, None, false, false),
            ],
        }],
        plain_text: "\u{65}\u{301} x\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };

    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    let glyph_columns: Vec<_> = input
        .glyph_cells
        .iter()
        .map(|cell| (cell.col_index, cell.text.to_string()))
        .collect();
    assert_eq!(
        glyph_columns,
        vec![(0, "\u{65}\u{301}".to_owned()), (2, "x".to_owned())]
    );
}

#[test]
fn terminal_paint_input_preserves_wide_cell_widths() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut wide = picker_cell("表", None, None, false, false);
    wide.width = 2;
    let mut spacer = picker_cell("", None, None, false, false);
    spacer.width = 0;
    let snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 3,
        rows: 1,
        scroll: test_terminal_scroll(1),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot::default(),
        rows_data: vec![octty_term::live::TerminalRowSnapshot {
            cells: vec![wide, spacer, picker_cell("x", None, None, false, false)],
        }],
        plain_text: "表 x\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };

    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    let glyph_cells: Vec<_> = input
        .glyph_cells
        .iter()
        .map(|cell| (cell.col_index, cell.cell_width, cell.text.to_string()))
        .collect();
    assert_eq!(
        glyph_cells,
        vec![(0, 2, "表".to_owned()), (2, 1, "x".to_owned())]
    );
}

#[test]
fn terminal_paint_input_moves_highlight_for_dirty_rows() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let marker_bg = TerminalRgb {
        r: 30,
        g: 90,
        b: 120,
    };
    let mut snapshot = TerminalGridSnapshot {
        session_id: "session-1".to_owned(),
        cols: 4,
        rows: 2,
        scroll: test_terminal_scroll(2),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: vec![0, 1],
            cells: 8,
        },
        rows_data: vec![
            octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell("a", None, Some(marker_bg), false, false),
                    picker_cell("b", None, Some(marker_bg), false, false),
                    picker_cell("c", None, Some(marker_bg), false, false),
                    picker_cell("d", None, Some(marker_bg), false, false),
                ],
            },
            octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell("w", None, None, false, false),
                    picker_cell("x", None, None, false, false),
                    picker_cell("y", None, None, false, false),
                    picker_cell("z", None, None, false, false),
                ],
            },
        ],
        plain_text: "abcd\nwxyz\n".to_owned(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    };
    let mut render_cache = TerminalRenderCache::default();

    let first = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );
    assert_eq!(first.rows_data[0].background_runs.len(), 1);
    assert!(first.rows_data[1].background_runs.is_empty());

    snapshot.damage = octty_term::live::TerminalDamageSnapshot {
        full: false,
        rows: vec![0, 1],
        cells: 8,
    };
    for cell in &mut snapshot.rows_data[0].cells {
        cell.bg = None;
    }
    for cell in &mut snapshot.rows_data[1].cells {
        cell.bg = Some(marker_bg);
    }
    let second = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(second.rebuilt_rows, 2);
    assert!(second.rows_data[0].background_runs.is_empty());
    assert_eq!(second.rows_data[1].background_runs.len(), 1);
    assert_eq!(second.rows_data[1].background_runs[0].start_col, 0);
    assert_eq!(second.rows_data[1].background_runs[0].cell_count, 4);
}

#[test]
fn terminal_picker_preview_workload_has_dense_runs_and_backgrounds() {
    let snapshot = picker_preview_snapshot(7, 120, 40);
    let mut render_cache = TerminalRenderCache::default();
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(snapshot.default_fg),
        terminal_rgb_to_rgba(snapshot.default_bg),
        &mut render_cache,
    );
    let background_runs: usize = input
        .rows_data
        .iter()
        .map(|row| row.background_runs.len())
        .sum();

    assert_eq!(input.cols, 120);
    assert_eq!(input.rows, 40);
    assert!(input.glyph_cells.len() > 1_000);
    assert!(background_runs > 40);
}

#[test]
fn terminal_picker_preview_reuses_dense_unchanged_rows() {
    let mut snapshot = picker_preview_snapshot(7, 120, 40);
    let mut render_cache = TerminalRenderCache::default();
    let _ = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(snapshot.default_fg),
        terminal_rgb_to_rgba(snapshot.default_bg),
        &mut render_cache,
    );

    snapshot.damage = octty_term::live::TerminalDamageSnapshot {
        full: false,
        rows: vec![10],
        cells: u32::from(snapshot.cols),
    };
    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(snapshot.default_fg),
        terminal_rgb_to_rgba(snapshot.default_bg),
        &mut render_cache,
    );

    assert_eq!(input.rebuilt_rows, 1);
    assert_eq!(input.reused_rows, 39);
    assert_eq!(
        input.repaint_backgrounds,
        terminal_row_background_submission_count(&input.rows_data[10])
    );
    assert!(input.glyph_cells.len() < 120);
    assert!(input.glyph_cells.iter().all(|cell| cell.row_index == 10));
}

#[test]
fn terminal_shell_keypress_repaints_one_row_payload() {
    let default_fg = TerminalRgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
    let mut snapshot =
        shell_prompt_snapshot("shell-keypress-row", "$ ", 2, 5, 24, default_fg, default_bg);
    let mut render_cache = TerminalRenderCache::default();
    let _ = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    snapshot.rows_data[2] = shell_row("$ a", snapshot.cols);
    snapshot.cursor = Some(octty_term::live::TerminalCursorSnapshot {
        col: 3,
        row: 2,
        visible: true,
    });
    snapshot.damage = octty_term::live::TerminalDamageSnapshot {
        full: false,
        rows: vec![2],
        cells: u32::from(snapshot.cols),
    };
    snapshot.plain_text = "$ a\n".to_owned();

    let input = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(default_fg),
        terminal_rgb_to_rgba(default_bg),
        &mut render_cache,
    );

    assert_eq!(input.rebuilt_rows, 1);
    assert_eq!(input.reused_rows, 4);
    assert_eq!(input.repaint_backgrounds, 1);
    assert_eq!(
        input
            .cursor
            .as_ref()
            .map(|cursor| (cursor.row_index, cursor.col_index)),
        Some((2, 3))
    );
    assert!(
        input
            .glyph_cells
            .iter()
            .all(|cell| cell.row_index == 2 && cell.col_index <= 2)
    );
}

#[test]
fn terminal_picker_preview_incremental_updates_stay_bounded() {
    let mut snapshot = picker_preview_snapshot(0, 120, 40);
    let mut render_cache = TerminalRenderCache::default();
    let _ = terminal_paint_input(
        &snapshot,
        terminal_rgb_to_rgba(snapshot.default_fg),
        terminal_rgb_to_rgba(snapshot.default_bg),
        &mut render_cache,
    );

    let mut paint_input_micros = Vec::new();
    let mut glyph_cells = Vec::new();
    let mut rebuilt_rows = Vec::new();
    for frame in 1..120 {
        let next = picker_preview_snapshot(frame, 120, 40);
        let dirty_row = (frame % (usize::from(snapshot.rows) - 1)) + 1;
        snapshot.rows_data[dirty_row] = next.rows_data[dirty_row].clone();
        snapshot.cursor = next.cursor;
        snapshot.damage = octty_term::live::TerminalDamageSnapshot {
            full: false,
            rows: vec![dirty_row as u16],
            cells: u32::from(snapshot.cols),
        };

        let started_at = Instant::now();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(snapshot.default_fg),
            terminal_rgb_to_rgba(snapshot.default_bg),
            &mut render_cache,
        );
        paint_input_micros.push(duration_micros(started_at.elapsed()));
        glyph_cells.push(input.glyph_cells.len() as u64);
        rebuilt_rows.push(input.rebuilt_rows as u64);
        assert_eq!(input.reused_rows, usize::from(snapshot.rows) - 1);
        assert!(
            input
                .glyph_cells
                .iter()
                .all(|cell| cell.row_index == dirty_row)
        );
        std::hint::black_box(input);
    }

    paint_input_micros.sort_unstable();
    glyph_cells.sort_unstable();
    rebuilt_rows.sort_unstable();
    let paint_input_p95 = latency_percentile(&paint_input_micros, 95);
    let glyph_cells_p95 = latency_percentile(&glyph_cells, 95);
    let rebuilt_rows_p95 = latency_percentile(&rebuilt_rows, 95);

    assert_eq!(rebuilt_rows_p95, 1);
    assert!(
        glyph_cells_p95 < u64::from(snapshot.cols),
        "dirty-row glyph payload p95 should stay below one full row, got {glyph_cells_p95}"
    );
    assert!(
        paint_input_p95 < 10_000,
        "paint-input p95 should stay below 10ms in debug builds, got {}",
        format_latency_micros(paint_input_p95)
    );
}

#[test]
#[ignore = "profiling workload; run with --ignored --nocapture"]
fn terminal_picker_preview_paint_input_profile() {
    let mut samples = VecDeque::new();
    let mut glyph_cells = VecDeque::new();
    let mut background_runs = VecDeque::new();
    let mut rebuilt_rows = VecDeque::new();
    let mut reused_rows = VecDeque::new();
    let mut render_cache = TerminalRenderCache::default();

    for frame in 0..240 {
        let snapshot = picker_preview_snapshot(frame, 120, 40);
        let started_at = Instant::now();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(snapshot.default_fg),
            terminal_rgb_to_rgba(snapshot.default_bg),
            &mut render_cache,
        );
        push_latency_sample(&mut samples, duration_micros(started_at.elapsed()));
        push_latency_sample(&mut glyph_cells, input.glyph_cells.len() as u64);
        push_latency_sample(&mut rebuilt_rows, input.rebuilt_rows as u64);
        push_latency_sample(&mut reused_rows, input.reused_rows as u64);
        push_latency_sample(
            &mut background_runs,
            input
                .rows_data
                .iter()
                .map(|row| row.background_runs.len())
                .sum::<usize>() as u64,
        );
        std::hint::black_box(input);
    }

    println!(
        "picker preview paint-input: {} · glyph cells {} · rebuilt rows {} · reused rows {} · background runs {}",
        latency_summary(&samples).unwrap(),
        count_summary(&glyph_cells).unwrap(),
        count_summary(&rebuilt_rows).unwrap(),
        count_summary(&reused_rows).unwrap(),
        count_summary(&background_runs).unwrap()
    );
}

#[test]
fn pane_navigation_moves_between_columns() {
    let mut snapshot = create_default_snapshot("workspace-1");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
    let first = snapshot.active_pane_id.clone().expect("first pane");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
    let second = snapshot.active_pane_id.clone().expect("second pane");

    snapshot.active_pane_id = Some(first.clone());
    assert_eq!(
        pane_navigation_target(&snapshot, PaneNavigationDirection::Right).as_deref(),
        Some(second.as_str())
    );

    snapshot.active_pane_id = Some(second);
    assert_eq!(
        pane_navigation_target(&snapshot, PaneNavigationDirection::Left).as_deref(),
        Some(first.as_str())
    );
}

#[test]
fn taskspace_viewport_offset_keeps_focused_column_visible() {
    let mut snapshot = create_default_snapshot("workspace-1");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
    let first = snapshot.active_pane_id.clone().expect("first pane");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
    let second = snapshot.active_pane_id.clone().expect("second pane");

    snapshot.active_pane_id = Some(first);
    assert_eq!(taskspace_viewport_offset(&snapshot, 560.0), 0.0);

    snapshot.active_pane_id = Some(second);
    assert_eq!(taskspace_viewport_offset(&snapshot, 560.0), 602.0);
}

#[test]
fn taskspace_viewport_offset_stays_zero_when_columns_fit() {
    let mut snapshot = create_default_snapshot("workspace-1");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));

    assert_eq!(taskspace_viewport_offset(&snapshot, 1_400.0), 0.0);
}

#[test]
fn terminal_resize_rows_subtracts_visible_chrome_without_perf_overlay() {
    let snapshot = add_pane(
        create_default_snapshot("workspace-1"),
        create_pane_state(PaneType::Shell, "/tmp", None),
    );

    let requests = terminal_resize_requests(Some(&snapshot), 1_000.0);

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].2, 90);
    assert_eq!(requests[0].3, 54);
}

#[test]
fn terminal_resize_from_viewport_height_matches_rendered_taskspace_chrome() {
    let snapshot = add_pane(
        create_default_snapshot("workspace-1"),
        create_pane_state(PaneType::Shell, "/tmp", None),
    );

    let requests =
        terminal_resize_requests(Some(&snapshot), taskspace_height_for_viewport(1_000.0));

    assert_eq!(taskspace_height_for_viewport(1_000.0), 976.0);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].2, 90);
    assert_eq!(requests[0].3, 53);
}

#[test]
fn terminal_scrollbar_geometry_tracks_viewport_offset() {
    let geometry = terminal_scrollbar_geometry(
        TerminalScrollSnapshot {
            total: 100,
            offset: 50,
            len: 25,
        },
        200.0,
    )
    .expect("scrollbar geometry");

    assert_eq!(
        geometry,
        TerminalScrollbarGeometry {
            track_height: 200.0,
            thumb_top: 100.0,
            thumb_height: 50.0,
        }
    );
    assert_eq!(
        terminal_scrollbar_geometry(
            TerminalScrollSnapshot {
                total: 25,
                offset: 0,
                len: 25,
            },
            200.0,
        ),
        None
    );
    assert_eq!(
        terminal_scrollbar_click_scroll_lines(
            TerminalScrollSnapshot {
                total: 100,
                offset: 50,
                len: 25,
            },
            25,
            200.0,
            20.0,
        ),
        Some(-24)
    );
    assert_eq!(
        terminal_scrollbar_click_scroll_lines(
            TerminalScrollSnapshot {
                total: 100,
                offset: 50,
                len: 25,
            },
            25,
            200.0,
            180.0,
        ),
        Some(24)
    );
    assert_eq!(
        terminal_scrollbar_click_scroll_lines(
            TerminalScrollSnapshot {
                total: 100,
                offset: 50,
                len: 25,
            },
            25,
            200.0,
            120.0,
        ),
        None
    );
}

#[test]
fn resize_focused_column_only_changes_active_column_width() {
    let mut snapshot = create_default_snapshot("workspace-1");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
    let first_column_id = snapshot.center_column_ids[0].clone();
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
    let second_column_id = snapshot.center_column_ids[1].clone();
    let first_width = snapshot.columns[&first_column_id].width_px;
    let second_width = snapshot.columns[&second_column_id].width_px;

    let resized = resize_focused_column_in_snapshot(&mut snapshot, ColumnResizeDirection::Wider)
        .expect("resized focused column");

    assert_eq!(snapshot.columns[&first_column_id].width_px, first_width);
    assert_eq!(resized, second_width + COLUMN_WIDTH_STEP_PX);
    assert_eq!(snapshot.columns[&second_column_id].width_px, resized);
}

#[test]
fn resize_focused_column_clamps_to_minimum_width() {
    let mut snapshot = create_default_snapshot("workspace-1");
    snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
    let column_id = snapshot.center_column_ids[0].clone();
    snapshot.columns.get_mut(&column_id).unwrap().width_px = MIN_COLUMN_WIDTH_PX;

    assert_eq!(
        resize_focused_column_in_snapshot(&mut snapshot, ColumnResizeDirection::Slimmer),
        None
    );
    assert_eq!(snapshot.columns[&column_id].width_px, MIN_COLUMN_WIDTH_PX);
}

fn picker_preview_snapshot(frame: usize, cols: u16, rows: u16) -> TerminalGridSnapshot {
    let default_fg = TerminalRgb {
        r: 210,
        g: 216,
        b: 222,
    };
    let default_bg = TerminalRgb {
        r: 18,
        g: 20,
        b: 22,
    };
    let mut rows_data = Vec::with_capacity(rows as usize);
    for row_index in 0..rows as usize {
        let mut cells = vec![picker_cell("", None, None, false, false); cols as usize];
        if row_index == 0 {
            write_picker_text(
                &mut cells,
                0,
                "  Find files                                      Preview",
                Some(TerminalRgb {
                    r: 240,
                    g: 240,
                    b: 240,
                }),
                Some(TerminalRgb {
                    r: 42,
                    g: 48,
                    b: 56,
                }),
                true,
                false,
            );
        } else {
            let selected = row_index == (frame % (rows as usize - 2)) + 1;
            let file_name = format!(
                " crates/octty-app/src/{:03}_picker_case.rs ",
                (frame + row_index) % 173
            );
            write_picker_text(
                &mut cells,
                0,
                &format!("{file_name:40}"),
                Some(if selected {
                    TerminalRgb {
                        r: 245,
                        g: 250,
                        b: 255,
                    }
                } else {
                    TerminalRgb {
                        r: 170,
                        g: 184,
                        b: 194,
                    }
                }),
                selected.then_some(TerminalRgb {
                    r: 28,
                    g: 92,
                    b: 72,
                }),
                selected,
                false,
            );
            write_picker_preview_line(&mut cells, row_index, frame, 43);
        }
        rows_data.push(octty_term::live::TerminalRowSnapshot { cells });
    }

    TerminalGridSnapshot {
        session_id: "picker-preview-profile".to_owned(),
        cols,
        rows,
        scroll: test_terminal_scroll(rows),
        default_fg,
        default_bg,
        cursor: Some(octty_term::live::TerminalCursorSnapshot {
            col: 2,
            row: ((frame % (rows as usize - 2)) + 1) as u16,
            visible: true,
        }),
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: (0..rows).collect(),
            cells: u32::from(cols) * u32::from(rows),
        },
        rows_data,
        plain_text: String::new(),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    }
}

fn shell_prompt_snapshot(
    session_id: &str,
    prompt: &str,
    cursor_col: u16,
    rows: u16,
    cols: u16,
    default_fg: TerminalRgb,
    default_bg: TerminalRgb,
) -> TerminalGridSnapshot {
    let mut rows_data = vec![
        octty_term::live::TerminalRowSnapshot {
            cells: vec![picker_cell("", None, None, false, false); cols as usize],
        };
        rows as usize
    ];
    rows_data[2] = shell_row(prompt, cols);

    TerminalGridSnapshot {
        session_id: session_id.to_owned(),
        cols,
        rows,
        scroll: test_terminal_scroll(rows),
        default_fg,
        default_bg,
        cursor: Some(octty_term::live::TerminalCursorSnapshot {
            col: cursor_col,
            row: 2,
            visible: true,
        }),
        damage: octty_term::live::TerminalDamageSnapshot {
            full: true,
            rows: (0..rows).collect(),
            cells: u32::from(cols) * u32::from(rows),
        },
        rows_data,
        plain_text: format!("{prompt}\n"),
        timing: octty_term::live::TerminalSnapshotTiming::default(),
    }
}

fn shell_row(text: &str, cols: u16) -> octty_term::live::TerminalRowSnapshot {
    let mut cells = vec![picker_cell("", None, None, false, false); cols as usize];
    write_picker_text(
        &mut cells,
        0,
        text,
        Some(TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        }),
        None,
        false,
        false,
    );
    octty_term::live::TerminalRowSnapshot { cells }
}

fn write_picker_preview_line(
    cells: &mut [octty_term::live::TerminalCellSnapshot],
    row_index: usize,
    frame: usize,
    start_col: usize,
) {
    let line_no = (row_index + frame) % 97;
    write_picker_text(
        cells,
        start_col,
        &format!("{line_no:>3} "),
        Some(TerminalRgb {
            r: 105,
            g: 116,
            b: 126,
        }),
        Some(TerminalRgb {
            r: 30,
            g: 34,
            b: 38,
        }),
        false,
        false,
    );
    let segments = [
        (
            "let ".to_owned(),
            TerminalRgb {
                r: 235,
                g: 118,
                b: 135,
            },
            false,
        ),
        (
            format!("preview_{line_no}"),
            TerminalRgb {
                r: 132,
                g: 204,
                b: 244,
            },
            false,
        ),
        (
            " = ".to_owned(),
            TerminalRgb {
                r: 210,
                g: 216,
                b: 222,
            },
            false,
        ),
        (
            format!("render_case({frame}, {row_index});"),
            TerminalRgb {
                r: 166,
                g: 218,
                b: 149,
            },
            false,
        ),
    ];
    let mut col = start_col + 4;
    for (text, color, bold) in segments {
        write_picker_text(cells, col, &text, Some(color), None, bold, false);
        col += text.chars().count();
    }
    if row_index % 5 == 0 {
        write_picker_text(
            cells,
            start_col + 5,
            " changed ",
            Some(TerminalRgb {
                r: 18,
                g: 20,
                b: 22,
            }),
            Some(TerminalRgb {
                r: 238,
                g: 212,
                b: 132,
            }),
            true,
            false,
        );
    }
}

fn write_picker_text(
    cells: &mut [octty_term::live::TerminalCellSnapshot],
    start_col: usize,
    text: &str,
    fg: Option<TerminalRgb>,
    bg: Option<TerminalRgb>,
    bold: bool,
    italic: bool,
) {
    for (offset, ch) in text.chars().enumerate() {
        let Some(cell) = cells.get_mut(start_col + offset) else {
            break;
        };
        *cell = picker_cell(&ch.to_string(), fg, bg, bold, italic);
    }
}

fn picker_cell(
    text: &str,
    fg: Option<TerminalRgb>,
    bg: Option<TerminalRgb>,
    bold: bool,
    italic: bool,
) -> octty_term::live::TerminalCellSnapshot {
    octty_term::live::TerminalCellSnapshot {
        text: text.to_owned(),
        width: 1,
        fg,
        bg,
        bold,
        italic,
        faint: false,
        blink: false,
        underline: false,
        inverse: false,
        invisible: false,
        strikethrough: false,
        overline: false,
    }
}

fn test_terminal_snapshot(
    session_id: &str,
    cols: u16,
    rows: u16,
    dirty_rows: Vec<u16>,
    full_damage: bool,
) -> TerminalGridSnapshot {
    let default_fg = TerminalRgb {
        r: 210,
        g: 216,
        b: 222,
    };
    let default_bg = TerminalRgb {
        r: 30,
        g: 34,
        b: 48,
    };
    let rows_data = (0..rows)
        .map(|_| octty_term::live::TerminalRowSnapshot {
            cells: (0..cols)
                .map(|_| picker_cell("", None, None, false, false))
                .collect(),
        })
        .collect();
    let damage_cells = dirty_rows.len().saturating_mul(usize::from(cols)) as u32;
    TerminalGridSnapshot {
        session_id: session_id.to_owned(),
        cols,
        rows,
        scroll: test_terminal_scroll(rows),
        default_fg,
        default_bg,
        cursor: None,
        damage: octty_term::live::TerminalDamageSnapshot {
            full: full_damage,
            rows: dirty_rows,
            cells: damage_cells,
        },
        rows_data,
        plain_text: String::new(),
        timing: octty_term::live::TerminalSnapshotTiming {
            dirty_rows: damage_cells / u32::from(cols),
            dirty_cells: damage_cells,
            ..Default::default()
        },
    }
}
