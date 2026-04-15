use super::*;
use std::fmt::Write as _;

#[test]
fn changed_terminal_waits_until_snapshot_interval() {
    let now = Instant::now();
    let last_snapshot_at = now - Duration::from_millis(10);

    assert_eq!(
        terminal_command_wait_timeout(
            true,
            false,
            MAX_INITIAL_SNAPSHOTS,
            Some(last_snapshot_at),
            now,
        ),
        Duration::from_millis(23)
    );
}

#[test]
fn forced_snapshot_is_due_immediately() {
    let now = Instant::now();

    assert!(terminal_snapshot_due(
        true,
        true,
        MAX_INITIAL_SNAPSHOTS,
        Some(now),
        now,
    ));
}

#[test]
fn idle_terminal_uses_idle_timeout() {
    assert_eq!(
        terminal_command_wait_timeout(false, false, MAX_INITIAL_SNAPSHOTS, None, Instant::now()),
        LIVE_TERMINAL_IDLE_TIMEOUT
    );
}

#[test]
fn runtime_input_drain_prioritizes_control_and_batches_pty_output() {
    let (command_tx, command_rx) = mpsc::channel();
    let (pty_output_tx, pty_output_rx) = mpsc::channel();

    pty_output_tx.send(b"abc".to_vec()).expect("first output");
    pty_output_tx.send(b"def".to_vec()).expect("second output");
    command_tx
        .send(LiveTerminalCommand::Scroll(1))
        .expect("control command");

    let drained = drain_terminal_runtime_inputs(&command_rx, &pty_output_rx);

    assert_eq!(drained.commands.len(), 1);
    assert!(matches!(
        drained.commands[0],
        LiveTerminalCommand::Scroll(1)
    ));
    assert_eq!(drained.pty_output.as_deref(), Some(&b"abcdef"[..]));
}

#[test]
fn runtime_input_drain_caps_pty_output_work_per_tick() {
    let (_command_tx, command_rx) = mpsc::channel();
    let (pty_output_tx, pty_output_rx) = mpsc::channel();

    for _ in 0..=MAX_PTY_OUTPUT_CHUNKS_PER_TICK {
        pty_output_tx.send(vec![b'x']).expect("output chunk");
    }

    let drained = drain_terminal_runtime_inputs(&command_rx, &pty_output_rx);

    assert!(drained.commands.is_empty());
    assert_eq!(
        drained.pty_output.as_ref().map(Vec::len),
        Some(MAX_PTY_OUTPUT_CHUNKS_PER_TICK)
    );
    assert_eq!(
        pty_output_rx.try_recv().expect("deferred chunk"),
        vec![b'x']
    );
}

#[test]
fn runtime_wake_drain_coalesces_wakeups() {
    let (wake_tx, wake_rx) = mpsc::channel();
    wake_tx
        .send(LiveTerminalWake::PtyOutput)
        .expect("output wake");
    wake_tx
        .send(LiveTerminalWake::Control)
        .expect("control wake");

    drain_terminal_wakes(&wake_rx);

    assert!(wake_rx.try_recv().is_err());
}

#[test]
fn picker_preview_ansi_fixture_reaches_snapshot() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 120,
        rows: 40,
        max_scrollback: 1_000,
    })
    .expect("terminal");
    terminal
        .resize(
            120,
            40,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let bytes = picker_preview_ansi_frame(3, 120, 40);
    terminal.vt_write(&bytes);

    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");
    let snapshot = renderer
        .snapshot("picker-preview-test", &terminal)
        .expect("snapshot");

    assert_eq!(snapshot.cols, 120);
    assert_eq!(snapshot.rows, 40);
    assert!(snapshot.plain_text.contains("preview_"));
    assert!(snapshot.timing.snapshot_cells >= 4_800);
    assert!(snapshot.timing.snapshot_text_cells > 1_000);
}

#[test]
fn replay_terminal_bytes_parses_escape_sequences() {
    let snapshot = replay_terminal_bytes(
        "replay-test",
        b"\x1b[2J\x1b[1;1Hplain \x1b[7mselected\x1b[0m",
        40,
        4,
    )
    .expect("replay snapshot");

    assert!(snapshot.plain_text.contains("plain selected"));
    assert!(!snapshot.plain_text.contains("\\x1b"));
    assert!(snapshot.rows_data[0].cells[6].inverse);
}

#[test]
fn terminal_trace_helpers_are_stable() {
    assert_eq!(terminal_trace_safe_name("a/b:c"), "a_b_c");
    assert_eq!(terminal_trace_hex_prefix(b"\x1b[7m", 8), "1b5b376d");
    assert_eq!(terminal_trace_hex_prefix(b"abcdef", 3), "616263...");
    assert_eq!(terminal_trace_rows(&[1, 3, 5]), "1,3,5");
}

#[test]
fn xtversion_query_does_not_emit_pty_input() {
    let grid_size = Cell::new((80, 24));
    let pty_responses = RefCell::new(VecDeque::<Vec<u8>>::new());
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal");
    install_terminal_effects(&mut terminal, &grid_size, &pty_responses).expect("terminal effects");

    terminal.vt_write(b"\x1b[>q");

    assert!(pty_responses.borrow().is_empty());
}

#[test]
fn key_encoder_emits_plain_space() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 100,
    })
    .expect("terminal");
    let mut input = KeyInputEncoder::new().expect("key encoder");

    let bytes = input
        .encode(
            &mut terminal,
            LiveTerminalKeyInput {
                key: LiveTerminalKey::Character(' '),
                text: Some(" ".to_owned()),
                unshifted: ' ',
                modifiers: LiveTerminalModifiers {
                    shift: false,
                    alt: false,
                    control: false,
                    platform: false,
                },
            },
        )
        .expect("encoded space");

    assert_eq!(bytes, b" ");
}

#[test]
fn snapshot_reports_incremental_dirty_rows_after_clean_extract() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 20,
        rows: 4,
        max_scrollback: 100,
    })
    .expect("terminal");
    terminal
        .resize(
            20,
            4,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");

    terminal.vt_write(b"initial");
    let _ = renderer
        .snapshot("dirty-row-test", &terminal)
        .expect("initial snapshot");
    renderer.mark_clean(&terminal).expect("mark clean");

    terminal.vt_write(b"\x1b[2;1Hx");
    let snapshot = renderer
        .snapshot("dirty-row-test", &terminal)
        .expect("incremental snapshot");

    assert!(!snapshot.damage.full);
    assert!(snapshot.damage.rows.contains(&1));
    assert!(snapshot.damage.rows.len() < usize::from(snapshot.rows));
    assert_eq!(
        snapshot.timing.dirty_rows,
        snapshot.damage.rows.len() as u32
    );
    assert_eq!(
        snapshot.timing.dirty_cells,
        snapshot.timing.dirty_rows * u32::from(snapshot.cols)
    );
}

#[test]
fn snapshot_does_not_dirty_rows_for_cursor_only_movement() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 20,
        rows: 4,
        max_scrollback: 100,
    })
    .expect("terminal");
    terminal
        .resize(
            20,
            4,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");

    terminal.vt_write(b"ab");
    let _ = renderer
        .snapshot("cursor-damage-test", &terminal)
        .expect("initial snapshot");
    renderer.mark_clean(&terminal).expect("mark clean");

    terminal.vt_write(b"\x1b[1;1H");
    let snapshot = renderer
        .snapshot("cursor-damage-test", &terminal)
        .expect("cursor-only snapshot");

    assert_eq!(
        snapshot
            .cursor
            .as_ref()
            .map(|cursor| (cursor.col, cursor.row)),
        Some((0, 0))
    );
    assert!(!snapshot.damage.full);
    assert!(snapshot.damage.rows.is_empty());
    assert_eq!(snapshot.damage.cells, 0);
}

#[test]
fn snapshot_reports_dirty_rows_when_style_marker_moves() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 20,
        rows: 4,
        max_scrollback: 100,
    })
    .expect("terminal");
    terminal
        .resize(
            20,
            4,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");

    terminal.vt_write(b"\x1b[2J\x1b[1;1H\x1b[2mfile-a\x1b[0m\x1b[2;1Hfile-b");
    let _ = renderer
        .snapshot("style-marker-test", &terminal)
        .expect("initial snapshot");
    renderer.mark_clean(&terminal).expect("mark clean");

    terminal.vt_write(b"\x1b[1;1H\x1b[0mfile-a\x1b[K\x1b[2;1H\x1b[2mfile-b\x1b[0m\x1b[K");
    let snapshot = renderer
        .snapshot("style-marker-test", &terminal)
        .expect("incremental snapshot");

    assert!(!snapshot.damage.full);
    assert!(snapshot.damage.rows.contains(&0));
    assert!(snapshot.damage.rows.contains(&1));
    assert!(!snapshot.rows_data[0].cells[0].faint);
    assert!(snapshot.rows_data[1].cells[0].faint);
}

#[test]
fn snapshot_reports_dirty_rows_when_background_marker_moves() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 20,
        rows: 4,
        max_scrollback: 100,
    })
    .expect("terminal");
    terminal
        .resize(
            20,
            4,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");

    terminal.vt_write(b"\x1b[2J\x1b[1;1H\x1b[48;2;30;90;120mfile-a\x1b[0m\x1b[2;1Hfile-b");
    let _ = renderer
        .snapshot("background-marker-test", &terminal)
        .expect("initial snapshot");
    renderer.mark_clean(&terminal).expect("mark clean");

    terminal
        .vt_write(b"\x1b[1;1H\x1b[0mfile-a\x1b[K\x1b[2;1H\x1b[48;2;30;90;120mfile-b\x1b[0m\x1b[K");
    let snapshot = renderer
        .snapshot("background-marker-test", &terminal)
        .expect("incremental snapshot");

    assert!(!snapshot.damage.full);
    assert!(snapshot.damage.rows.contains(&0));
    assert!(snapshot.damage.rows.contains(&1));
    assert_eq!(snapshot.rows_data[0].cells[0].bg, None);
    assert_eq!(
        snapshot.rows_data[1].cells[0].bg,
        Some(TerminalRgb {
            r: 30,
            g: 90,
            b: 120,
        })
    );
}

#[test]
#[ignore = "profiling workload; run with --ignored --nocapture"]
fn picker_preview_vt_pipeline_profile() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 120,
        rows: 40,
        max_scrollback: 10_000,
    })
    .expect("terminal");
    terminal
        .resize(
            120,
            40,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .expect("resize");
    let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");
    let mut vt_write = VecDeque::new();
    let mut update = VecDeque::new();
    let mut extract = VecDeque::new();
    let mut build = VecDeque::new();
    let mut bytes_per_frame = VecDeque::new();
    let mut text_cells = VecDeque::new();
    let mut dirty_rows = VecDeque::new();
    let mut dirty_cells = VecDeque::new();

    for frame in 0..180 {
        let bytes = picker_preview_ansi_frame(frame, 120, 40);
        push_test_sample(&mut bytes_per_frame, bytes.len() as u64);
        let vt_started_at = Instant::now();
        terminal.vt_write(&bytes);
        push_test_sample(&mut vt_write, micros_since(vt_started_at, Instant::now()));

        let snapshot_started_at = Instant::now();
        let snapshot = renderer
            .snapshot("picker-preview-profile", &terminal)
            .expect("snapshot");
        push_test_sample(
            &mut build,
            micros_since(snapshot_started_at, Instant::now()),
        );
        push_test_sample(&mut update, snapshot.timing.snapshot_update_micros);
        push_test_sample(&mut extract, snapshot.timing.snapshot_extract_micros);
        push_test_sample(
            &mut text_cells,
            u64::from(snapshot.timing.snapshot_text_cells),
        );
        push_test_sample(&mut dirty_rows, u64::from(snapshot.timing.dirty_rows));
        push_test_sample(&mut dirty_cells, u64::from(snapshot.timing.dirty_cells));
        std::hint::black_box(snapshot);
    }

    println!(
        "picker preview VT pipeline: bytes {} · vt {} · update {} · extract {} · build {} · dirty rows {} · dirty cells {} · text cells {}",
        test_count_summary(&bytes_per_frame),
        test_summary(&vt_write),
        test_summary(&update),
        test_summary(&extract),
        test_summary(&build),
        test_count_summary(&dirty_rows),
        test_count_summary(&dirty_cells),
        test_count_summary(&text_cells)
    );
}

fn picker_preview_ansi_frame(frame: usize, cols: u16, rows: u16) -> Vec<u8> {
    let mut out = String::from("\x1b[?1049h\x1b[?25l\x1b[H");
    let preview_start = 44usize;
    for row in 0..rows as usize {
        let _ = write!(out, "\x1b[{};1H\x1b[0m", row + 1);
        if row == 0 {
            let _ = write!(
                out,
                "\x1b[48;2;42;48;56m\x1b[38;2;240;240;240m{:width$}",
                "  Find files                                      Preview",
                width = cols as usize
            );
            continue;
        }

        let selected = row == (frame % (rows as usize - 2)) + 1;
        if selected {
            out.push_str("\x1b[48;2;28;92;72m\x1b[38;2;245;250;255m\x1b[1m");
        } else {
            out.push_str("\x1b[0m\x1b[38;2;170;184;194m");
        }
        let _ = write!(
            out,
            "{:<width$}",
            format!(
                " crates/octty-app/src/{:03}_picker_case.rs ",
                (frame + row) % 173
            ),
            width = preview_start - 2
        );
        out.push_str("\x1b[0m  ");
        let line_no = (row + frame) % 97;
        let _ = write!(
            out,
            "\x1b[38;2;105;116;126m{line_no:>3} \
                 \x1b[38;2;235;118;135mlet \
                 \x1b[38;2;132;204;244mpreview_{line_no}\
                 \x1b[38;2;210;216;222m = \
                 \x1b[38;2;166;218;149mrender_case({frame}, {row});"
        );
        if row % 5 == 0 {
            let _ = write!(
                out,
                "\x1b[{};{}H\x1b[48;2;238;212;132m\x1b[38;2;18;20;22m\x1b[1m changed ",
                row + 1,
                preview_start + 5
            );
        }
        out.push_str("\x1b[0m");
    }
    out.into_bytes()
}

fn push_test_sample(samples: &mut VecDeque<u64>, micros: u64) {
    if samples.len() == 512 {
        samples.pop_front();
    }
    samples.push_back(micros);
}

fn test_summary(samples: &VecDeque<u64>) -> String {
    let mut sorted: Vec<_> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = sorted[(sorted.len().saturating_sub(1) * 50) / 100];
    let p95 = sorted[(sorted.len().saturating_sub(1) * 95) / 100];
    let max = sorted.last().copied().unwrap_or_default();
    format!(
        "p50 {} p95 {} max {}",
        test_format_micros(p50),
        test_format_micros(p95),
        test_format_micros(max)
    )
}

fn test_count_summary(samples: &VecDeque<u64>) -> String {
    let mut sorted: Vec<_> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = sorted[(sorted.len().saturating_sub(1) * 50) / 100];
    let p95 = sorted[(sorted.len().saturating_sub(1) * 95) / 100];
    let max = sorted.last().copied().unwrap_or_default();
    format!("p50 {p50} p95 {p95} max {max}")
}

fn test_format_micros(micros: u64) -> String {
    if micros >= 1_000 {
        format!("{:.1}ms", micros as f64 / 1_000.0)
    } else {
        format!("{micros}us")
    }
}
