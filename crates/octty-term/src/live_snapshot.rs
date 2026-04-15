use super::*;

pub(crate) struct SnapshotExtractor<'alloc> {
    render_state: RenderState<'alloc>,
    row_iter: RowIterator<'alloc>,
    cell_iter: CellIterator<'alloc>,
    previous_row_hashes: Vec<u64>,
    previous_size: Option<(u16, u16)>,
}

impl<'alloc> SnapshotExtractor<'alloc> {
    pub(crate) fn new() -> Result<Self, TerminalError> {
        Ok(Self {
            render_state: RenderState::new().map_err(renderer_error)?,
            row_iter: RowIterator::new().map_err(renderer_error)?,
            cell_iter: CellIterator::new().map_err(renderer_error)?,
            previous_row_hashes: Vec::new(),
            previous_size: None,
        })
    }

    pub(crate) fn snapshot(
        &mut self,
        session_id: &str,
        terminal: &Terminal<'alloc, '_>,
    ) -> Result<TerminalGridSnapshot, TerminalError> {
        let update_started_at = Instant::now();
        let snapshot = self
            .render_state
            .update(terminal)
            .map_err(renderer_context("render state update"))?;
        let snapshot_update_micros = micros_since(update_started_at, Instant::now());
        let extract_started_at = Instant::now();
        let colors = snapshot.colors().map_err(renderer_context("read colors"))?;
        let default_fg = terminal_rgb(colors.foreground);
        let default_bg = terminal_rgb(colors.background);
        let cursor = if snapshot
            .cursor_visible()
            .map_err(renderer_context("read cursor visibility"))?
        {
            snapshot
                .cursor_viewport()
                .map_err(renderer_context("read cursor viewport"))?
                .map(|viewport| TerminalCursorSnapshot {
                    col: viewport.x,
                    row: viewport.y,
                    visible: true,
                })
        } else {
            None
        };
        let cols = snapshot.cols().map_err(renderer_context("read cols"))?;
        let rows = snapshot.rows().map_err(renderer_context("read rows"))?;
        let mut rows_data = Vec::with_capacity(rows as usize);
        let mut plain_text = String::new();
        let mut row_iteration = self
            .row_iter
            .update(&snapshot)
            .map_err(renderer_context("update row iterator"))?;
        let mut snapshot_cells = 0u32;
        let mut snapshot_text_cells = 0u32;
        let mut dirty_rows = Vec::new();
        let mut row_hashes = Vec::with_capacity(rows as usize);
        let size_changed = self.previous_size != Some((cols, rows));

        let mut row_index = 0u16;
        while let Some(row) = row_iteration.next() {
            let mut cells = Vec::with_capacity(cols as usize);
            let mut row_text = String::new();
            let mut cell_iteration = self
                .cell_iter
                .update(row)
                .map_err(renderer_context("update cell iterator"))?;
            while let Some(cell) = cell_iteration.next() {
                let graphemes = cell
                    .graphemes()
                    .map_err(renderer_context("read cell graphemes"))?;
                let text: String = graphemes.into_iter().collect();
                let style = cell.style().map_err(renderer_context("read cell style"))?;
                let width = match cell
                    .raw_cell()
                    .and_then(|raw| raw.wide())
                    .map_err(renderer_context("read cell width"))?
                {
                    libghostty_vt::screen::CellWide::Narrow => 1,
                    libghostty_vt::screen::CellWide::Wide => 2,
                    libghostty_vt::screen::CellWide::SpacerTail
                    | libghostty_vt::screen::CellWide::SpacerHead => 0,
                };
                let fg = cell
                    .fg_color()
                    .map_err(renderer_context("read cell foreground"))?
                    .map(terminal_rgb);
                let bg = cell
                    .bg_color()
                    .map_err(renderer_context("read cell background"))?
                    .map(terminal_rgb);
                snapshot_cells = snapshot_cells.saturating_add(1);
                if !text.is_empty() {
                    snapshot_text_cells = snapshot_text_cells.saturating_add(1);
                }
                if text.is_empty() {
                    row_text.push(' ');
                } else {
                    row_text.push_str(&text);
                }
                cells.push(TerminalCellSnapshot {
                    text,
                    width,
                    fg,
                    bg,
                    bold: style.bold,
                    italic: style.italic,
                    faint: style.faint,
                    blink: style.blink,
                    underline: !matches!(style.underline, libghostty_vt::style::Underline::None),
                    inverse: style.inverse,
                    invisible: style.invisible,
                    strikethrough: style.strikethrough,
                    overline: style.overline,
                });
            }
            drop(cell_iteration);
            let row_hash = terminal_row_hash(&cells);
            let row_changed = size_changed
                || self
                    .previous_row_hashes
                    .get(row_index as usize)
                    .is_none_or(|previous_hash| *previous_hash != row_hash);
            if row_changed {
                dirty_rows.push(row_index);
            }
            row_hashes.push(row_hash);
            plain_text.push_str(row_text.trim_end());
            plain_text.push('\n');
            rows_data.push(TerminalRowSnapshot { cells });
            row.set_dirty(false)
                .map_err(renderer_context("clear row dirty state"))?;
            row_index = row_index.saturating_add(1);
        }
        dirty_rows.sort_unstable();
        dirty_rows.dedup();
        snapshot
            .set_dirty(Dirty::Clean)
            .map_err(renderer_context("clear global dirty state"))?;
        let snapshot_extract_micros = micros_since(extract_started_at, Instant::now());
        let dirty_row_count = dirty_rows.len() as u32;
        let dirty_cells = dirty_row_count.saturating_mul(u32::from(cols));
        let full_damage = dirty_row_count == u32::from(rows);
        self.previous_row_hashes = row_hashes;
        self.previous_size = Some((cols, rows));

        Ok(TerminalGridSnapshot {
            session_id: session_id.to_owned(),
            cols,
            rows,
            default_fg,
            default_bg,
            cursor,
            damage: TerminalDamageSnapshot {
                full: full_damage,
                rows: dirty_rows,
                cells: dirty_cells,
            },
            rows_data,
            plain_text,
            timing: TerminalSnapshotTiming {
                snapshot_update_micros,
                snapshot_extract_micros,
                snapshot_cells,
                snapshot_text_cells,
                dirty_rows: dirty_row_count,
                dirty_cells,
                ..TerminalSnapshotTiming::default()
            },
        })
    }

    pub(crate) fn mark_clean(
        &mut self,
        terminal: &Terminal<'alloc, '_>,
    ) -> Result<(), TerminalError> {
        let snapshot = self.render_state.update(terminal).map_err(renderer_error)?;
        snapshot.set_dirty(Dirty::Clean).map_err(renderer_error)
    }
}

pub(crate) fn terminal_row_hash(cells: &[TerminalCellSnapshot]) -> u64 {
    let mut hasher = DefaultHasher::new();
    cells.hash(&mut hasher);
    hasher.finish()
}
