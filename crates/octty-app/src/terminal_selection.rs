fn terminal_paste_bytes(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
}

fn terminal_clipboard_paste_text(clipboard: &ClipboardItem) -> anyhow::Result<Option<String>> {
    if let Some(image) = clipboard.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) if !image.bytes.is_empty() => Some(image),
        _ => None,
    }) {
        let path = write_clipboard_image_to_temp_file(image)?;
        return Ok(Some(quote_terminal_path_for_paste(
            path.to_string_lossy().as_ref(),
        )));
    }

    Ok(clipboard.text())
}

fn write_clipboard_image_to_temp_file(image: &Image) -> anyhow::Result<PathBuf> {
    static CLIPBOARD_IMAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

    let directory = std::env::temp_dir().join("octty-clipboard");
    fs::create_dir_all(&directory)?;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = CLIPBOARD_IMAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let extension = image_format_extension(image.format);
    let file_path = directory.join(format!(
        "clipboard-image-{now_ms}-{}-{counter}.{extension}",
        std::process::id()
    ));
    fs::write(&file_path, &image.bytes)?;
    Ok(file_path)
}

fn image_format_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

fn quote_terminal_path_for_paste(path: &str) -> String {
    if path.chars().all(is_safe_shell_path_char) {
        return path.to_owned();
    }

    format!("'{}'", path.replace('\'', "'\\''"))
}

fn is_safe_shell_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '_' | '.' | '/' | ':' | '@' | '%' | '+' | '=' | ',' | '-'
        )
}

fn terminal_grid_point_from_mouse_position(
    position: Point<Pixels>,
    interaction: &TerminalGridInteractionState,
    cols: u16,
    rows: u16,
) -> Option<TerminalGridPoint> {
    let bounds = interaction.bounds?;
    if cols == 0 || rows == 0 || !bounds.contains(&position) {
        return None;
    }
    Some(terminal_grid_point_from_local_position(
        position.relative_to(&bounds.origin),
        cols,
        rows,
    ))
}

fn terminal_grid_point_from_local_position(
    position: Point<Pixels>,
    cols: u16,
    rows: u16,
) -> TerminalGridPoint {
    let col = ((f32::from(position.x) / TERMINAL_CELL_WIDTH).floor() as i32)
        .clamp(0, i32::from(cols.saturating_sub(1))) as u16;
    let row = ((f32::from(position.y) / TERMINAL_CELL_HEIGHT).floor() as i32)
        .clamp(0, i32::from(rows.saturating_sub(1))) as u16;
    TerminalGridPoint { row, col }
}

fn terminal_selection_mode_from_modifiers(modifiers: Modifiers) -> TerminalSelectionMode {
    TerminalSelectionMode {
        rectangular: modifiers.control,
        filter_indent: modifiers.shift,
    }
}

fn terminal_selection_runs(
    selection: &TerminalSelection,
    cols: u16,
    rows: u16,
) -> Vec<TerminalSelectionRun> {
    if cols == 0 || rows == 0 || selection.anchor == selection.active {
        return Vec::new();
    }
    if selection.mode.rectangular {
        return terminal_rectangular_selection_runs(selection, cols, rows);
    }
    let (start, end) = terminal_selection_ordered_points(selection);
    let start_row = start.row.min(rows.saturating_sub(1));
    let end_row = end.row.min(rows.saturating_sub(1));
    (start_row..=end_row)
        .filter_map(|row| {
            let start_col = if row == start_row { start.col } else { 0 }.min(cols);
            let end_col = if row == end_row {
                end.col.saturating_add(1)
            } else {
                cols
            }
            .min(cols);
            (end_col > start_col).then_some(TerminalSelectionRun {
                row,
                start_col,
                end_col,
            })
        })
        .collect()
}

fn terminal_rectangular_selection_runs(
    selection: &TerminalSelection,
    cols: u16,
    rows: u16,
) -> Vec<TerminalSelectionRun> {
    let start_row = selection.anchor.row.min(selection.active.row);
    let end_row = selection.anchor.row.max(selection.active.row);
    let start_col = selection.anchor.col.min(selection.active.col).min(cols);
    let end_col = selection
        .anchor
        .col
        .max(selection.active.col)
        .saturating_add(1)
        .min(cols);

    if end_col <= start_col {
        return Vec::new();
    }

    (start_row.min(rows.saturating_sub(1))..=end_row.min(rows.saturating_sub(1)))
        .map(|row| TerminalSelectionRun {
            row,
            start_col,
            end_col,
        })
        .collect()
}

fn terminal_selection_ordered_points(
    selection: &TerminalSelection,
) -> (TerminalGridPoint, TerminalGridPoint) {
    let a = selection.anchor;
    let b = selection.active;
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn terminal_selection_text(
    snapshot: &TerminalGridSnapshot,
    selection: &TerminalSelection,
) -> String {
    let text = terminal_selection_text_unfiltered(snapshot, selection);
    if selection.mode.filter_indent {
        terminal_selection_text_remove_common_indent(&text)
    } else {
        text
    }
}

fn terminal_selection_text_unfiltered(
    snapshot: &TerminalGridSnapshot,
    selection: &TerminalSelection,
) -> String {
    let runs = terminal_selection_runs(selection, snapshot.cols, snapshot.rows);
    let mut lines = Vec::with_capacity(runs.len());
    for run in runs {
        let Some(row) = snapshot.rows_data.get(run.row as usize) else {
            continue;
        };
        let mut line = String::new();
        for col in run.start_col..run.end_col {
            let Some(cell) = row.cells.get(col as usize) else {
                continue;
            };
            if cell.width == 0 || cell.invisible {
                continue;
            }
            if cell.text.is_empty() {
                line.push(' ');
            } else {
                line.push_str(&cell.text);
            }
        }
        if selection.mode.rectangular {
            lines.push(line);
        } else {
            lines.push(line.trim_end().to_owned());
        }
    }
    lines.join("\n")
}

fn terminal_selection_text_remove_common_indent(text: &str) -> String {
    let lines = text.split('\n').collect::<Vec<_>>();
    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    if common_indent == 0 {
        return text.to_owned();
    }

    lines
        .into_iter()
        .map(|line| {
            let mut stripped = 0usize;
            let mut byte_index = line.len();
            for (index, ch) in line.char_indices() {
                if stripped == common_indent || (ch != ' ' && ch != '\t') {
                    byte_index = index;
                    break;
                }
                stripped += 1;
                byte_index = index + ch.len_utf8();
            }
            &line[byte_index..]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn write_terminal_primary_text(text: String, cx: &mut Context<OcttyApp>) {
    if !text.is_empty() {
        cx.write_to_primary(ClipboardItem::new_string(text));
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn write_terminal_primary_text(_text: String, _cx: &mut Context<OcttyApp>) {}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn read_terminal_primary_text(cx: &mut Context<OcttyApp>) -> Option<String> {
    cx.read_from_primary()?.text()
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn read_terminal_primary_text(_cx: &mut Context<OcttyApp>) -> Option<String> {
    None
}
