fn terminal_input_from_key_event(event: &KeyDownEvent) -> Option<TerminalInput> {
    live_terminal_input_from_key_parts(
        &event.keystroke.key,
        event.keystroke.key_char.as_deref(),
        event.keystroke.modifiers.control,
        event.keystroke.modifiers.alt,
        event.keystroke.modifiers.shift,
        event.keystroke.modifiers.platform,
        event.keystroke.modifiers.function,
    )
    .map(TerminalInput::LiveKey)
}

fn workspace_shortcut_index_from_key_event(event: &KeyDownEvent) -> Option<usize> {
    workspace_shortcut_index_from_key_parts(
        &event.keystroke.key,
        event.keystroke.key_char.as_deref(),
        event.keystroke.modifiers.control,
        event.keystroke.modifiers.alt,
        event.keystroke.modifiers.shift,
        event.keystroke.modifiers.platform,
        event.keystroke.modifiers.function,
    )
}

fn workspace_shortcut_index_from_key_parts(
    key: &str,
    key_char: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    function: bool,
) -> Option<usize> {
    if !control || !shift || alt || platform || function {
        return None;
    }

    workspace_shortcut_index_from_token(key)
        .or_else(|| key_char.and_then(workspace_shortcut_index_from_token))
}

fn workspace_shortcut_index_from_token(token: &str) -> Option<usize> {
    match token {
        "1" | "!" => Some(0),
        "2" | "@" => Some(1),
        "3" | "#" => Some(2),
        "4" | "$" => Some(3),
        "5" | "%" => Some(4),
        "6" | "^" => Some(5),
        "7" | "&" => Some(6),
        "8" | "*" => Some(7),
        "9" | "(" => Some(8),
        "0" | ")" => Some(9),
        _ => None,
    }
}

fn live_terminal_input_from_key_parts(
    key: &str,
    key_char: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    function: bool,
) -> Option<LiveTerminalKeyInput> {
    if function {
        return None;
    }
    if workspace_shortcut_index_from_key_parts(key, key_char, control, alt, shift, platform, false)
        .is_some()
    {
        return None;
    }
    if control && shift && is_clipboard_action_key(key) {
        return None;
    }
    if control && shift && is_pane_action_key(key) {
        return None;
    }
    if control && alt && is_column_resize_key(key) {
        return None;
    }

    let normalized_key = key.to_ascii_lowercase();
    if let Some(key_text) = terminal_printable_key_text(key_char, control, platform) {
        return Some(live_terminal_printable_input(
            key_text, control, alt, shift, platform,
        ));
    }
    if let Some(key_text) =
        synthesized_terminal_printable_key_text(&normalized_key, control, shift, platform)
    {
        return Some(live_terminal_printable_input(
            key_text, control, alt, shift, platform,
        ));
    }

    let live_key = match normalized_key.as_str() {
        "enter" => LiveTerminalKey::Enter,
        "backspace" => LiveTerminalKey::Backspace,
        "delete" => LiveTerminalKey::Delete,
        "tab" => LiveTerminalKey::Tab,
        "escape" => LiveTerminalKey::Escape,
        "left" => LiveTerminalKey::ArrowLeft,
        "right" => LiveTerminalKey::ArrowRight,
        "up" => LiveTerminalKey::ArrowUp,
        "down" => LiveTerminalKey::ArrowDown,
        "home" => LiveTerminalKey::Home,
        "end" => LiveTerminalKey::End,
        "pageup" => LiveTerminalKey::PageUp,
        "pagedown" => LiveTerminalKey::PageDown,
        "insert" => LiveTerminalKey::Insert,
        "space" => LiveTerminalKey::Space,
        "f1" => LiveTerminalKey::F(1),
        "f2" => LiveTerminalKey::F(2),
        "f3" => LiveTerminalKey::F(3),
        "f4" => LiveTerminalKey::F(4),
        "f5" => LiveTerminalKey::F(5),
        "f6" => LiveTerminalKey::F(6),
        "f7" => LiveTerminalKey::F(7),
        "f8" => LiveTerminalKey::F(8),
        "f9" => LiveTerminalKey::F(9),
        "f10" => LiveTerminalKey::F(10),
        "f11" => LiveTerminalKey::F(11),
        "f12" => LiveTerminalKey::F(12),
        _ if normalized_key.len() == 1 => LiveTerminalKey::Character(
            normalized_key
                .chars()
                .next()
                .map(unshifted_character)
                .unwrap_or('\0'),
        ),
        _ => return None,
    };

    let unshifted = match live_key {
        LiveTerminalKey::Character(character) => character,
        LiveTerminalKey::Space => ' ',
        _ => '\0',
    };

    Some(LiveTerminalKeyInput {
        key: live_key,
        text: None,
        modifiers: LiveTerminalModifiers {
            shift,
            alt,
            control,
            platform,
        },
        unshifted,
    })
}

fn live_terminal_printable_input(
    text: String,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
) -> LiveTerminalKeyInput {
    let character = text.chars().next().map(unshifted_character).unwrap_or('\0');
    LiveTerminalKeyInput {
        key: LiveTerminalKey::Character(character),
        text: Some(text),
        modifiers: LiveTerminalModifiers {
            shift,
            alt,
            control,
            platform,
        },
        unshifted: character,
    }
}

fn terminal_printable_key_text(
    key_char: Option<&str>,
    control: bool,
    platform: bool,
) -> Option<String> {
    if control || platform {
        return None;
    }
    let text = key_char?;
    if text.is_empty()
        || text == "\r"
        || text == "\n"
        || text.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(text.to_owned())
}

fn synthesized_terminal_printable_key_text(
    normalized_key: &str,
    control: bool,
    shift: bool,
    platform: bool,
) -> Option<String> {
    if control || platform {
        return None;
    }
    if normalized_key == "space" {
        return Some(" ".to_owned());
    }
    if !shift && normalized_key.len() == 1 {
        return Some(normalized_key.to_owned());
    }
    None
}

fn is_pane_action_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "left"
            | "right"
            | "up"
            | "down"
            | "arrowleft"
            | "arrowright"
            | "arrowup"
            | "arrowdown"
            | "w"
    )
}

fn is_column_resize_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "left" | "right" | "arrowleft" | "arrowright"
    )
}

fn is_clipboard_action_key(key: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "c" | "x" | "v")
}

fn unshifted_character(character: char) -> char {
    match character {
        'A'..='Z' => character.to_ascii_lowercase(),
        ')' => '0',
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        other => other,
    }
}

fn active_terminal_pane_id(snapshot: &WorkspaceSnapshot) -> Option<String> {
    snapshot
        .active_pane_id
        .as_deref()
        .and_then(|pane_id| {
            snapshot
                .panes
                .get(pane_id)
                .filter(|pane| matches!(pane.payload, PanePayload::Terminal(_)))
                .map(|pane| pane.id.clone())
        })
        .or_else(|| {
            snapshot
                .panes
                .values()
                .find(|pane| matches!(pane.payload, PanePayload::Terminal(_)))
                .map(|pane| pane.id.clone())
        })
}

fn pane_navigation_target(
    snapshot: &WorkspaceSnapshot,
    direction: PaneNavigationDirection,
) -> Option<String> {
    let active_pane_id = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| first_center_pane_id(snapshot))?;

    let (column_index, pane_index) = pane_layout_position(snapshot, active_pane_id)?;
    let target = match direction {
        PaneNavigationDirection::Left => column_index
            .checked_sub(1)
            .and_then(|index| pane_in_neighbor_column(snapshot, index, pane_index)),
        PaneNavigationDirection::Right => {
            pane_in_neighbor_column(snapshot, column_index + 1, pane_index)
        }
    };

    target.cloned()
}

fn first_center_pane_id(snapshot: &WorkspaceSnapshot) -> Option<&str> {
    snapshot
        .center_column_ids
        .iter()
        .filter_map(|column_id| snapshot.columns.get(column_id))
        .flat_map(|column| column.pane_ids.iter())
        .next()
        .map(String::as_str)
}

fn pane_layout_position(snapshot: &WorkspaceSnapshot, pane_id: &str) -> Option<(usize, usize)> {
    for (column_index, column_id) in snapshot.center_column_ids.iter().enumerate() {
        let column = snapshot.columns.get(column_id)?;
        if let Some(pane_index) = column.pane_ids.iter().position(|id| id == pane_id) {
            return Some((column_index, pane_index));
        }
    }
    None
}

fn center_column(
    snapshot: &WorkspaceSnapshot,
    column_index: usize,
) -> Option<&octty_core::WorkspaceColumn> {
    snapshot
        .center_column_ids
        .get(column_index)
        .and_then(|column_id| snapshot.columns.get(column_id))
}

fn pane_in_neighbor_column(
    snapshot: &WorkspaceSnapshot,
    column_index: usize,
    source_pane_index: usize,
) -> Option<&String> {
    let column = center_column(snapshot, column_index)?;
    let target_index = source_pane_index.min(column.pane_ids.len().saturating_sub(1));
    column.pane_ids.get(target_index)
}

fn resize_focused_column_in_snapshot(
    snapshot: &mut WorkspaceSnapshot,
    direction: ColumnResizeDirection,
) -> Option<f64> {
    let column_id = active_column_id(snapshot)?;
    let column = snapshot.columns.get_mut(&column_id)?;
    let delta = match direction {
        ColumnResizeDirection::Slimmer => -COLUMN_WIDTH_STEP_PX,
        ColumnResizeDirection::Wider => COLUMN_WIDTH_STEP_PX,
    };
    let next_width = (column.width_px + delta).clamp(MIN_COLUMN_WIDTH_PX, MAX_COLUMN_WIDTH_PX);
    if (next_width - column.width_px).abs() < f64::EPSILON {
        return None;
    }
    column.width_px = next_width;
    snapshot.updated_at = now_ms();
    Some(next_width)
}

fn active_column_id(snapshot: &WorkspaceSnapshot) -> Option<String> {
    let active_pane_id = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| first_center_pane_id(snapshot))?;
    snapshot
        .center_column_ids
        .iter()
        .find(|column_id| {
            snapshot.columns.get(*column_id).is_some_and(|column| {
                column
                    .pane_ids
                    .iter()
                    .any(|pane_id| pane_id == active_pane_id)
            })
        })
        .cloned()
}

fn preview_terminal_input(snapshot: &mut WorkspaceSnapshot, pane_id: &str, input: &TerminalInput) {
    let Some(pane) = snapshot.panes.get_mut(pane_id) else {
        return;
    };
    let PanePayload::Terminal(payload) = &mut pane.payload else {
        return;
    };

    match input {
        TerminalInput::LiveKey(key_input) if key_input.text.is_some() => {
            payload
                .restored_buffer
                .push_str(key_input.text.as_deref().unwrap_or_default());
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Enter => {
            payload.restored_buffer.push('\n');
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Backspace => {
            payload.restored_buffer.pop();
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Tab => {
            payload.restored_buffer.push('\t');
        }
        TerminalInput::LiveKey(_) => {}
    }
    snapshot.updated_at = now_ms();
}
