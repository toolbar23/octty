use super::*;

pub(crate) struct LiveTerminalRuntime {
    pub(crate) spec: TerminalSessionSpec,
    pub(crate) session_id: String,
    pub(crate) master: Box<dyn portable_pty::MasterPty + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) child: Box<dyn portable_pty::Child + Send + Sync>,
    pub(crate) startup_command: Option<Vec<u8>>,
    pub(crate) command_rx: mpsc::Receiver<LiveTerminalCommand>,
    pub(crate) pty_output_rx: mpsc::Receiver<Vec<u8>>,
    pub(crate) wake_rx: mpsc::Receiver<LiveTerminalWake>,
    pub(crate) snapshot_tx: mpsc::Sender<TerminalGridSnapshot>,
    pub(crate) notification_tx: mpsc::Sender<TerminalNotification>,
    pub(crate) exit_tx: mpsc::Sender<LiveTerminalExit>,
    pub(crate) snapshot_notifier: LiveTerminalSnapshotNotifier,
}

impl LiveTerminalRuntime {
    pub(crate) fn run(mut self) -> Result<(), TerminalError> {
        let grid_size = Cell::new((self.spec.cols, self.spec.rows));
        let pty_responses = RefCell::new(VecDeque::<Vec<u8>>::new());
        let mut terminal = Terminal::new(TerminalOptions {
            cols: self.spec.cols,
            rows: self.spec.rows,
            max_scrollback: 10_000,
        })
        .map_err(renderer_error)?;

        terminal
            .resize(
                self.spec.cols,
                self.spec.rows,
                u32::from(DEFAULT_CELL_WIDTH),
                u32::from(DEFAULT_CELL_HEIGHT),
            )
            .map_err(renderer_error)?;
        install_terminal_effects(
            &mut terminal,
            &grid_size,
            &pty_responses,
            self.notification_tx.clone(),
        )?;

        let mut renderer = SnapshotExtractor::new()?;
        let mut input = KeyInputEncoder::new()?;
        let mut terminal_changed = true;
        let mut force_snapshot = true;
        let mut emitted_snapshots = 0usize;
        let mut last_snapshot_at: Option<Instant> = None;
        let mut last_pty_output_at: Option<Instant> = None;
        let mut interactive_output_until: Option<Instant> = None;
        let mut pending_vt_write_micros = 0u64;
        let mut pending_pty_output_bytes = 0u64;
        let mut notification_parser = TerminalOscNotificationParser::default();
        let mut osc_passthrough_filter = TerminalOscPassthroughFilter::default();
        let mut recorder =
            TerminalTraceRecorder::from_env(&self.session_id, self.spec.cols, self.spec.rows);
        let mut startup_command = self.startup_command.take();

        loop {
            let timeout = terminal_command_wait_timeout(
                terminal_changed,
                force_snapshot,
                emitted_snapshots,
                last_snapshot_at,
                Instant::now(),
            );
            match self.wake_rx.recv_timeout(timeout) {
                Ok(_) => drain_terminal_wakes(&self.wake_rx),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }

            let drained_inputs =
                drain_terminal_runtime_inputs(&self.command_rx, &self.pty_output_rx);

            for command in drained_inputs.commands {
                let is_interactive_input = matches!(
                    &command,
                    LiveTerminalCommand::Key(_) | LiveTerminalCommand::Bytes(_)
                );
                let command_received_at = Instant::now();
                let effect = self.handle_command(
                    command,
                    &mut terminal,
                    &mut input,
                    &grid_size,
                    recorder.as_mut(),
                )?;
                if effect.shutdown {
                    return Ok(());
                }
                pending_vt_write_micros =
                    pending_vt_write_micros.saturating_add(effect.vt_write_micros);
                pending_pty_output_bytes =
                    pending_pty_output_bytes.saturating_add(effect.pty_output_bytes);
                drain_pty_responses(&mut self.writer, &pty_responses, recorder.as_mut())?;
                if is_interactive_input {
                    interactive_output_until =
                        Some(command_received_at + LIVE_TERMINAL_INTERACTIVE_OUTPUT_WINDOW);
                }
                terminal_changed |= effect.terminal_changed;
                force_snapshot |= effect.force_snapshot;
            }

            if let Some(pty_output) = drained_inputs.pty_output {
                let output_received_at = Instant::now();
                let effect = self.handle_pty_output(
                    pty_output,
                    &mut notification_parser,
                    &mut osc_passthrough_filter,
                    &mut terminal,
                    recorder.as_mut(),
                    &mut startup_command,
                )?;
                pending_vt_write_micros =
                    pending_vt_write_micros.saturating_add(effect.vt_write_micros);
                pending_pty_output_bytes =
                    pending_pty_output_bytes.saturating_add(effect.pty_output_bytes);
                drain_pty_responses(&mut self.writer, &pty_responses, recorder.as_mut())?;
                last_pty_output_at.get_or_insert(output_received_at);
                if interactive_output_until.is_some_and(|deadline| output_received_at <= deadline) {
                    force_snapshot = true;
                }
                terminal_changed |= effect.terminal_changed;
                force_snapshot |= effect.force_snapshot;
            }

            let now = Instant::now();
            if terminal_snapshot_due(
                terminal_changed,
                force_snapshot,
                emitted_snapshots,
                last_snapshot_at,
                now,
            ) {
                let snapshot_started_at = Instant::now();
                let mut snapshot = renderer.snapshot(&self.session_id, &terminal)?;
                let snapshot_finished_at = Instant::now();
                snapshot.timing = TerminalSnapshotTiming {
                    pty_to_snapshot_micros: last_pty_output_at
                        .map(|output_at| micros_since(output_at, snapshot_started_at)),
                    vt_write_micros: pending_vt_write_micros,
                    pty_output_bytes: pending_pty_output_bytes,
                    snapshot_update_micros: snapshot.timing.snapshot_update_micros,
                    snapshot_extract_micros: snapshot.timing.snapshot_extract_micros,
                    snapshot_build_micros: micros_since(snapshot_started_at, snapshot_finished_at),
                    snapshot_cells: snapshot.timing.snapshot_cells,
                    snapshot_text_cells: snapshot.timing.snapshot_text_cells,
                    dirty_rows: snapshot.timing.dirty_rows,
                    dirty_cells: snapshot.timing.dirty_cells,
                };
                if let Some(recorder) = recorder.as_mut() {
                    recorder.record_snapshot(&snapshot);
                }
                if self.snapshot_tx.send(snapshot).is_ok() {
                    self.snapshot_notifier.notify();
                }
                terminal_changed = false;
                force_snapshot = false;
                last_pty_output_at = None;
                pending_vt_write_micros = 0;
                pending_pty_output_bytes = 0;
                last_snapshot_at = Some(snapshot_started_at);
                emitted_snapshots += 1;
                if emitted_snapshots >= MAX_INITIAL_SNAPSHOTS {
                    renderer.mark_clean(&terminal)?;
                }
            }

            if let Ok(Some(status)) = self.child.try_wait() {
                let _ = self.exit_tx.send(LiveTerminalExit {
                    session_id: self.session_id.clone(),
                    exit_code: Some(i64::from(status.exit_code())),
                });
                self.snapshot_notifier.notify();
                return Ok(());
            }
        }
    }

    fn handle_command<'a>(
        &mut self,
        command: LiveTerminalCommand,
        terminal: &mut Terminal<'a, 'a>,
        input: &mut KeyInputEncoder<'a>,
        grid_size: &Cell<(u16, u16)>,
        mut recorder: Option<&mut TerminalTraceRecorder>,
    ) -> Result<LiveTerminalCommandEffect, TerminalError> {
        match command {
            LiveTerminalCommand::Key(key_input) => {
                let bytes = input.encode(terminal, key_input)?;
                if !bytes.is_empty() {
                    if let Some(recorder) = recorder.as_mut() {
                        recorder.record_input("key", &bytes);
                    }
                    self.writer.write_all(&bytes)?;
                    self.writer.flush()?;
                }
                Ok(LiveTerminalCommandEffect::unchanged())
            }
            LiveTerminalCommand::Bytes(bytes) => {
                if let Some(recorder) = recorder.as_mut() {
                    recorder.record_input("bytes", &bytes);
                }
                self.writer.write_all(&bytes)?;
                self.writer.flush()?;
                Ok(LiveTerminalCommandEffect::unchanged())
            }
            LiveTerminalCommand::Resize(resize) => {
                let cols = resize.cols.max(1);
                let rows = resize.rows.max(1);
                grid_size.set((cols, rows));
                terminal
                    .resize(
                        cols,
                        rows,
                        u32::from(DEFAULT_CELL_WIDTH),
                        u32::from(DEFAULT_CELL_HEIGHT),
                    )
                    .map_err(renderer_error)?;
                self.master
                    .resize(PtySize {
                        cols,
                        rows,
                        pixel_width: resize.pixel_width,
                        pixel_height: resize.pixel_height,
                    })
                    .map_err(|error| TerminalError::Pty(error.to_string()))?;
                if let Some(recorder) = recorder.as_mut() {
                    recorder.record_resize(cols, rows, resize.pixel_width, resize.pixel_height);
                }
                Ok(LiveTerminalCommandEffect::changed_now())
            }
            LiveTerminalCommand::Scroll(lines) => {
                terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Delta(lines));
                if let Some(recorder) = recorder.as_mut() {
                    recorder.record_event("scroll", &format!("lines={lines}"));
                }
                Ok(LiveTerminalCommandEffect::changed_now())
            }
            LiveTerminalCommand::Shutdown => Ok(LiveTerminalCommandEffect {
                terminal_changed: false,
                force_snapshot: false,
                shutdown: true,
                vt_write_micros: 0,
                pty_output_bytes: 0,
            }),
        }
    }

    fn handle_pty_output<'a>(
        &mut self,
        bytes: Vec<u8>,
        notification_parser: &mut TerminalOscNotificationParser,
        osc_passthrough_filter: &mut TerminalOscPassthroughFilter,
        terminal: &mut Terminal<'a, 'a>,
        mut recorder: Option<&mut TerminalTraceRecorder>,
        startup_command: &mut Option<Vec<u8>>,
    ) -> Result<LiveTerminalCommandEffect, TerminalError> {
        let byte_count = bytes.len() as u64;
        if let Some(recorder) = recorder.as_mut() {
            recorder.record_output(&bytes);
        }
        for notification in notification_parser.push(&bytes) {
            let _ = self.notification_tx.send(notification);
        }
        let vt_bytes = osc_passthrough_filter.push(&bytes);
        if startup_command.is_some()
            && bytes
                .windows(b"[retach: new session".len())
                .any(|window| window == b"[retach: new session")
        {
            if let Some(command) = startup_command.take() {
                self.writer.write_all(&command)?;
                self.writer.flush()?;
                if let Some(recorder) = recorder.as_mut() {
                    recorder.record_input("startup", &command);
                }
            }
        }
        let vt_write_started_at = Instant::now();
        terminal.vt_write(&vt_bytes);
        Ok(LiveTerminalCommandEffect::changed_with_vt_write(
            micros_since(vt_write_started_at, Instant::now()),
            byte_count,
        ))
    }
}

pub(crate) struct TerminalRuntimeDrainedInputs {
    pub(crate) commands: Vec<LiveTerminalCommand>,
    pub(crate) pty_output: Option<Vec<u8>>,
}

pub(crate) fn drain_terminal_runtime_inputs(
    command_rx: &mpsc::Receiver<LiveTerminalCommand>,
    pty_output_rx: &mpsc::Receiver<Vec<u8>>,
) -> TerminalRuntimeDrainedInputs {
    let mut commands = Vec::new();
    while commands.len() < MAX_CONTROL_COMMANDS_PER_TICK {
        match command_rx.try_recv() {
            Ok(command) => commands.push(command),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }

    let mut pty_output = Vec::new();
    let mut output_chunks = 0usize;
    while output_chunks < MAX_PTY_OUTPUT_CHUNKS_PER_TICK {
        match pty_output_rx.try_recv() {
            Ok(bytes) => {
                output_chunks += 1;
                pty_output.extend_from_slice(&bytes);
                if pty_output.len() >= MAX_PTY_OUTPUT_BYTES_PER_TICK {
                    break;
                }
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }

    TerminalRuntimeDrainedInputs {
        commands,
        pty_output: (!pty_output.is_empty()).then_some(pty_output),
    }
}

pub(crate) fn drain_terminal_wakes(wake_rx: &mpsc::Receiver<LiveTerminalWake>) {
    while wake_rx.try_recv().is_ok() {}
}

struct LiveTerminalCommandEffect {
    terminal_changed: bool,
    force_snapshot: bool,
    shutdown: bool,
    vt_write_micros: u64,
    pty_output_bytes: u64,
}

impl LiveTerminalCommandEffect {
    fn changed_with_vt_write(vt_write_micros: u64, pty_output_bytes: u64) -> Self {
        Self {
            terminal_changed: true,
            force_snapshot: false,
            shutdown: false,
            vt_write_micros,
            pty_output_bytes,
        }
    }

    fn changed_now() -> Self {
        Self {
            terminal_changed: true,
            force_snapshot: true,
            shutdown: false,
            vt_write_micros: 0,
            pty_output_bytes: 0,
        }
    }

    fn unchanged() -> Self {
        Self {
            terminal_changed: false,
            force_snapshot: false,
            shutdown: false,
            vt_write_micros: 0,
            pty_output_bytes: 0,
        }
    }
}

pub(crate) fn terminal_command_wait_timeout(
    terminal_changed: bool,
    force_snapshot: bool,
    emitted_snapshots: usize,
    last_snapshot_at: Option<Instant>,
    now: Instant,
) -> Duration {
    if !terminal_changed {
        return LIVE_TERMINAL_IDLE_TIMEOUT;
    }
    if force_snapshot || emitted_snapshots < MAX_INITIAL_SNAPSHOTS {
        return Duration::ZERO;
    }
    last_snapshot_at
        .map(|last_snapshot_at| {
            LIVE_TERMINAL_SNAPSHOT_INTERVAL
                .saturating_sub(now.saturating_duration_since(last_snapshot_at))
        })
        .unwrap_or(Duration::ZERO)
}

pub(crate) fn terminal_snapshot_due(
    terminal_changed: bool,
    force_snapshot: bool,
    emitted_snapshots: usize,
    last_snapshot_at: Option<Instant>,
    now: Instant,
) -> bool {
    terminal_changed
        && terminal_command_wait_timeout(
            terminal_changed,
            force_snapshot,
            emitted_snapshots,
            last_snapshot_at,
            now,
        )
        .is_zero()
}

pub(crate) fn install_terminal_effects<'a>(
    terminal: &mut Terminal<'a, 'a>,
    grid_size: &'a Cell<(u16, u16)>,
    pty_responses: &'a RefCell<VecDeque<Vec<u8>>>,
    notification_tx: mpsc::Sender<TerminalNotification>,
) -> Result<(), TerminalError> {
    terminal
        .on_pty_write(|_terminal, data| {
            if !terminal_pty_response_is_xtversion(data) {
                pty_responses.borrow_mut().push_back(data.to_vec());
            }
        })
        .map_err(renderer_error)?
        .on_bell(move |_terminal| {
            let _ = notification_tx.send(TerminalNotification {
                title: "Terminal needs attention".to_owned(),
                body: "A terminal emitted a bell.".to_owned(),
            });
        })
        .map_err(renderer_error)?
        .on_size(move |_terminal| {
            let (columns, rows) = grid_size.get();
            Some(SizeReportSize {
                rows,
                columns,
                cell_width: u32::from(DEFAULT_CELL_WIDTH),
                cell_height: u32::from(DEFAULT_CELL_HEIGHT),
            })
        })
        .map_err(renderer_error)?
        .on_device_attributes(|_terminal| {
            Some(DeviceAttributes {
                primary: PrimaryDeviceAttributes::new(
                    ConformanceLevel::VT220,
                    [
                        DeviceAttributeFeature::COLUMNS_132,
                        DeviceAttributeFeature::SELECTIVE_ERASE,
                        DeviceAttributeFeature::ANSI_COLOR,
                    ],
                ),
                secondary: SecondaryDeviceAttributes {
                    device_type: DeviceType::VT220,
                    firmware_version: 1,
                    rom_cartridge: 0,
                },
                tertiary: Default::default(),
            })
        })
        .map_err(renderer_error)?
        .on_color_scheme(|_terminal| None)
        .map_err(renderer_error)?;
    Ok(())
}

fn terminal_pty_response_is_xtversion(data: &[u8]) -> bool {
    data.starts_with(b"\x1bP>|") && data.ends_with(b"\x1b\\")
}

fn drain_pty_responses(
    writer: &mut Box<dyn Write + Send>,
    pty_responses: &RefCell<VecDeque<Vec<u8>>>,
    mut recorder: Option<&mut TerminalTraceRecorder>,
) -> Result<(), TerminalError> {
    while let Some(response) = pty_responses.borrow_mut().pop_front() {
        if let Some(recorder) = recorder.as_mut() {
            recorder.record_input("pty-response", &response);
        }
        writer.write_all(&response)?;
    }
    writer.flush()?;
    Ok(())
}
