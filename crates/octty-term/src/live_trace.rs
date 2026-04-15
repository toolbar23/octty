use super::*;

pub(crate) struct TerminalTraceRecorder {
    started_at: Instant,
    output: File,
    events: File,
    output_offset: u64,
}

impl TerminalTraceRecorder {
    pub(crate) fn from_env(session_id: &str, cols: u16, rows: u16) -> Option<Self> {
        let dir = std::env::var_os("OCTTY_TERMINAL_RECORD_DIR")?;
        let dir = PathBuf::from(dir);
        if let Err(error) = create_dir_all(&dir) {
            eprintln!(
                "[octty-term] failed to create terminal record dir `{}`: {error}",
                dir.display()
            );
            return None;
        }

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let basename = format!("{stamp}-{}", terminal_trace_safe_name(session_id));
        let output_path = dir.join(format!("{basename}.pty"));
        let events_path = dir.join(format!("{basename}.events"));
        let output = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output_path)
        {
            Ok(file) => file,
            Err(error) => {
                eprintln!(
                    "[octty-term] failed to create terminal output record `{}`: {error}",
                    output_path.display()
                );
                return None;
            }
        };
        let events = match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&events_path)
        {
            Ok(file) => file,
            Err(error) => {
                eprintln!(
                    "[octty-term] failed to create terminal event record `{}`: {error}",
                    events_path.display()
                );
                return None;
            }
        };

        let mut recorder = Self {
            started_at: Instant::now(),
            output,
            events,
            output_offset: 0,
        };
        recorder.record_event(
            "start",
            &format!(
                "session={session_id} cols={cols} rows={rows} output={}",
                output_path.display()
            ),
        );
        Some(recorder)
    }

    pub(crate) fn record_output(&mut self, bytes: &[u8]) {
        let offset = self.output_offset;
        if self.output.write_all(bytes).is_ok() {
            let _ = self.output.flush();
            self.output_offset = self.output_offset.saturating_add(bytes.len() as u64);
        }
        self.record_event(
            "output",
            &format!(
                "offset={offset} len={} hex={}",
                bytes.len(),
                terminal_trace_hex_prefix(bytes, 48)
            ),
        );
    }

    pub(crate) fn record_input(&mut self, source: &str, bytes: &[u8]) {
        self.record_event(
            "input",
            &format!(
                "source={source} len={} hex={}",
                bytes.len(),
                terminal_trace_hex_prefix(bytes, 48)
            ),
        );
    }

    pub(crate) fn record_resize(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) {
        self.record_event(
            "resize",
            &format!(
                "cols={cols} rows={rows} pixel_width={pixel_width} pixel_height={pixel_height}"
            ),
        );
    }

    pub(crate) fn record_snapshot(&mut self, snapshot: &TerminalGridSnapshot) {
        let cursor = snapshot
            .cursor
            .as_ref()
            .map(|cursor| format!("{},{}", cursor.col, cursor.row))
            .unwrap_or_else(|| "none".to_owned());
        self.record_event(
            "snapshot",
            &format!(
                "cols={} rows={} cursor={} damage_full={} dirty_rows={} dirty_cells={} text_cells={} output_offset={}",
                snapshot.cols,
                snapshot.rows,
                cursor,
                snapshot.damage.full,
                terminal_trace_rows(&snapshot.damage.rows),
                snapshot.damage.cells,
                snapshot.timing.snapshot_text_cells,
                self.output_offset
            ),
        );
    }

    pub(crate) fn record_event(&mut self, kind: &str, detail: &str) {
        let micros = self.started_at.elapsed().as_micros();
        let _ = writeln!(self.events, "{micros} kind={kind} {detail}");
        let _ = self.events.flush();
    }
}

pub(crate) fn terminal_trace_safe_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "terminal".to_owned()
    } else {
        safe
    }
}

pub(crate) fn terminal_trace_hex_prefix(bytes: &[u8], max_bytes: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let len = bytes.len().min(max_bytes);
    let mut encoded = String::with_capacity(len.saturating_mul(2) + 3);
    for byte in &bytes[..len] {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    if bytes.len() > max_bytes {
        encoded.push_str("...");
    }
    encoded
}

pub(crate) fn terminal_trace_rows(rows: &[u16]) -> String {
    let mut output = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&row.to_string());
    }
    output
}
