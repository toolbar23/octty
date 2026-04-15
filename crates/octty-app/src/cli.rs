pub fn run() {
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("create tokio runtime"));
    if std::env::args().any(|arg| arg == "--headless-check") {
        runtime
            .block_on(load_bootstrap(false))
            .expect("load Rust Octty bootstrap");
        println!("octty headless check ok");
        return;
    }
    if std::env::args().any(|arg| arg == "--bootstrap-check") {
        let bootstrap = runtime
            .block_on(load_bootstrap(true))
            .expect("load Rust Octty bootstrap");
        println!(
            "octty bootstrap check ok: {} workspace(s)",
            bootstrap.workspaces.len()
        );
        return;
    }
    if std::env::args().any(|arg| arg == "--pane-check") {
        let count = runtime
            .block_on(pane_persistence_check())
            .expect("run pane persistence check");
        println!("octty pane check ok: {count} pane(s)");
        return;
    }
    if std::env::args().any(|arg| arg == "--shell-check") {
        let session_id = runtime
            .block_on(shell_session_check())
            .expect("run shell session check");
        println!("octty shell check ok: {session_id}");
        return;
    }
    if std::env::args().any(|arg| arg == "--terminal-io-check") {
        let marker = runtime
            .block_on(terminal_io_check())
            .expect("run terminal io check");
        println!("octty terminal io check ok: {marker}");
        return;
    }
    if std::env::args().any(|arg| arg == "--live-terminal-check") {
        let marker = runtime
            .block_on(live_terminal_check())
            .expect("run live terminal check");
        println!("octty live terminal check ok: {marker}");
        return;
    }
    if let Some((events_path, output_path)) = terminal_replay_events_args() {
        let summary =
            terminal_replay_events_check(events_path, output_path).expect("replay terminal events");
        println!("{summary}");
        return;
    }
    if let Some((path, cols, rows)) = terminal_replay_record_args() {
        let summary =
            terminal_replay_record_check(path, cols, rows).expect("replay terminal record");
        println!("{summary}");
        return;
    }

    let bootstrap = runtime
        .block_on(load_bootstrap(true))
        .unwrap_or_else(|error| BootstrapState {
            status: format!("Startup failed: {error:#}"),
            project_roots: Vec::new(),
            workspaces: Vec::new(),
            active_workspace_index: None,
            active_snapshot: None,
            pane_activity: Vec::new(),
        });

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        gpui_tokio::init_from_runtime(cx, runtime.clone());
        cx.bind_keys(workspace_key_bindings());
        set_workspace_menu(cx, &bootstrap.workspaces);

        let bounds = Bounds::centered(None, size(px(1200.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Octty".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window);
                let view = cx.new(|cx| OcttyApp::new(bootstrap, focus_handle, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("open Octty window");
        cx.activate(true);
    });
}

async fn pane_persistence_check() -> anyhow::Result<usize> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let snapshot = add_pane(
        snapshot,
        create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None),
    );

    let store = TursoStore::open(default_store_path()).await?;
    store.save_snapshot(&snapshot).await?;
    let saved = load_workspace_snapshot(&store, workspace).await?;
    Ok(saved.panes.len())
}

async fn shell_session_check() -> anyhow::Result<String> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let pane = create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None);
    let pane_id = pane.id.clone();
    let snapshot = add_pane(snapshot, pane);
    let snapshot = start_terminal_session(
        &TursoStore::open(default_store_path()).await?,
        workspace,
        snapshot,
        &pane_id,
    )
    .await?;
    Ok(snapshot
        .panes
        .get(&pane_id)
        .and_then(|pane| match &pane.payload {
            PanePayload::Terminal(payload) => payload.session_id.clone(),
            _ => None,
        })
        .unwrap_or_default())
}

async fn terminal_io_check() -> anyhow::Result<String> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let pane = create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None);
    let pane_id = pane.id.clone();
    let snapshot = add_pane(snapshot, pane);
    let store = TursoStore::open(default_store_path()).await?;
    let mut snapshot = start_terminal_session(&store, workspace, snapshot, &pane_id).await?;

    let payload = terminal_payload_for_pane(&snapshot, &pane_id)?.clone();
    let spec = terminal_spec_for_payload(workspace, &pane_id, &payload, 120, 40);
    resize_tmux_session(&spec, 120, 40).await?;

    let marker = format!("octty-terminal-io-{}", now_ms());
    let session_id = ensure_tmux_session(&spec).await?;
    send_tmux_text(&spec, &format!("clear; printf '{marker}\\n'")).await?;
    send_tmux_keys(&spec, &["Enter"]).await?;
    let screen = capture_tmux_until_contains(&spec, &marker, Duration::from_millis(1_000)).await?;
    snapshot =
        persist_terminal_screen(&store, workspace, snapshot, &pane_id, session_id, screen).await?;
    store.save_snapshot(&snapshot).await?;

    Ok(marker)
}

async fn live_terminal_check() -> anyhow::Result<String> {
    let marker = format!("octty-live-terminal-{}", now_ms());
    let pane_id = format!("pane-{}", now_ms());
    let spec = TerminalSessionSpec {
        workspace_id: "live-terminal-check".to_owned(),
        pane_id,
        kind: octty_core::TerminalKind::Shell,
        cwd: std::env::current_dir()?.to_string_lossy().to_string(),
        cols: 80,
        rows: 24,
    };
    let mut terminal = spawn_live_terminal(spec)?;
    terminal.send_bytes(format!("printf '{marker}\\n'\r").into_bytes())?;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(2_000);
    loop {
        for snapshot in terminal.drain_snapshots() {
            if snapshot.plain_text.contains(&marker) {
                let session_id = terminal.session_id().to_owned();
                drop(terminal);
                let _ = kill_tmux_session(&session_id).await;
                return Ok(marker);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let session_id = terminal.session_id().to_owned();
            drop(terminal);
            let _ = kill_tmux_session(&session_id).await;
            anyhow::bail!("live terminal snapshot did not contain marker `{marker}`");
        }
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
}

fn terminal_replay_record_args() -> Option<(PathBuf, u16, u16)> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--terminal-replay-record" {
            let path = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--terminal-replay-record requires a .pty path");
                std::process::exit(2);
            });
            let cols = args
                .next()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(120);
            let rows = args
                .next()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(40);
            return Some((path, cols, rows));
        }
    }
    None
}

fn terminal_replay_events_args() -> Option<(PathBuf, Option<PathBuf>)> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--terminal-replay-events" {
            let events_path = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--terminal-replay-events requires a .events path");
                std::process::exit(2);
            });
            return Some((events_path, args.next().map(PathBuf::from)));
        }
    }
    None
}

fn terminal_replay_record_check(path: PathBuf, cols: u16, rows: u16) -> anyhow::Result<String> {
    let bytes = std::fs::read(&path)?;
    let snapshot = replay_terminal_bytes("terminal-replay", &bytes, cols, rows)?;
    Ok(terminal_replay_summary(
        "octty terminal replay ok",
        &path,
        bytes.len(),
        &snapshot,
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalReplayEventsPlan {
    output_path: PathBuf,
    initial_cols: u16,
    initial_rows: u16,
    steps: Vec<TerminalReplayEventsStep>,
}

#[derive(Debug, PartialEq, Eq)]
enum TerminalReplayEventsStep {
    Output { offset: usize, len: usize },
    Resize { cols: u16, rows: u16 },
}

fn terminal_replay_events_check(
    events_path: PathBuf,
    output_path_override: Option<PathBuf>,
) -> anyhow::Result<String> {
    let events = std::fs::read_to_string(&events_path)?;
    let mut plan = parse_terminal_replay_events(&events)?;
    if let Some(output_path) = output_path_override {
        plan.output_path = output_path;
    }
    let bytes = std::fs::read(&plan.output_path)?;
    let mut steps = Vec::with_capacity(plan.steps.len());
    for step in &plan.steps {
        match *step {
            TerminalReplayEventsStep::Resize { cols, rows } => {
                steps.push(TerminalReplayStep::Resize { cols, rows });
            }
            TerminalReplayEventsStep::Output { offset, len } => {
                let end = offset
                    .checked_add(len)
                    .ok_or_else(|| anyhow::anyhow!("output offset overflow at {offset}+{len}"))?;
                let chunk = bytes.get(offset..end).ok_or_else(|| {
                    anyhow::anyhow!(
                        "output chunk {offset}..{end} is outside {} bytes",
                        bytes.len()
                    )
                })?;
                steps.push(TerminalReplayStep::Output(chunk));
            }
        }
    }
    let snapshot = replay_terminal_steps(
        "terminal-replay-events",
        plan.initial_cols,
        plan.initial_rows,
        steps,
    )?;
    Ok(terminal_replay_summary(
        "octty terminal event replay ok",
        &events_path,
        bytes.len(),
        &snapshot,
    ))
}

fn parse_terminal_replay_events(events: &str) -> anyhow::Result<TerminalReplayEventsPlan> {
    let mut output_path = None;
    let mut initial_cols = None;
    let mut initial_rows = None;
    let mut steps = Vec::new();

    for line in events.lines() {
        match terminal_trace_value(line, "kind") {
            Some("start") => {
                output_path = Some(PathBuf::from(
                    terminal_trace_value(line, "output")
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing output path"))?,
                ));
                initial_cols = Some(
                    terminal_trace_value(line, "cols")
                        .and_then(parse_u16)
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing cols"))?,
                );
                initial_rows = Some(
                    terminal_trace_value(line, "rows")
                        .and_then(parse_u16)
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing rows"))?,
                );
            }
            Some("resize") => {
                let cols = terminal_trace_value(line, "cols")
                    .and_then(parse_u16)
                    .ok_or_else(|| anyhow::anyhow!("trace resize is missing cols"))?;
                let rows = terminal_trace_value(line, "rows")
                    .and_then(parse_u16)
                    .ok_or_else(|| anyhow::anyhow!("trace resize is missing rows"))?;
                steps.push(TerminalReplayEventsStep::Resize { cols, rows });
            }
            Some("output") => {
                let offset = terminal_trace_value(line, "offset")
                    .and_then(parse_usize)
                    .ok_or_else(|| anyhow::anyhow!("trace output is missing offset"))?;
                let len = terminal_trace_value(line, "len")
                    .and_then(parse_usize)
                    .ok_or_else(|| anyhow::anyhow!("trace output is missing len"))?;
                steps.push(TerminalReplayEventsStep::Output { offset, len });
            }
            _ => {}
        }
    }

    Ok(TerminalReplayEventsPlan {
        output_path: output_path.ok_or_else(|| anyhow::anyhow!("trace is missing start event"))?,
        initial_cols: initial_cols.ok_or_else(|| anyhow::anyhow!("trace is missing start cols"))?,
        initial_rows: initial_rows.ok_or_else(|| anyhow::anyhow!("trace is missing start rows"))?,
        steps,
    })
}

fn terminal_trace_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
}

fn parse_u16(value: &str) -> Option<u16> {
    value.parse().ok()
}

fn parse_usize(value: &str) -> Option<usize> {
    value.parse().ok()
}

fn terminal_replay_summary(
    label: &str,
    path: &Path,
    bytes_len: usize,
    snapshot: &TerminalGridSnapshot,
) -> String {
    let cursor = snapshot
        .cursor
        .as_ref()
        .map(|cursor| format!("{},{}", cursor.col, cursor.row))
        .unwrap_or_else(|| "none".to_owned());
    format!(
        "{label}: path={} bytes={} grid={}x{} cursor={} dirty_rows={} dirty_cells={}\n{}\n{}",
        path.display(),
        bytes_len,
        snapshot.cols,
        snapshot.rows,
        cursor,
        snapshot.damage.rows.len(),
        snapshot.damage.cells,
        snapshot.plain_text,
        terminal_replay_style_summary(snapshot)
    )
}

fn terminal_replay_style_summary(snapshot: &TerminalGridSnapshot) -> String {
    let mut lines = Vec::new();
    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        let bg_runs = terminal_replay_bg_runs(row, snapshot.default_bg);
        if bg_runs.len() <= 1
            && bg_runs
                .first()
                .is_none_or(|run| run.color == snapshot.default_bg)
        {
            continue;
        }

        let text = row
            .cells
            .iter()
            .map(|cell| {
                if cell.text.is_empty() {
                    " "
                } else {
                    cell.text.as_str()
                }
            })
            .collect::<String>()
            .trim_end()
            .to_owned();
        lines.push(format!(
            "style row {:02}: bg={} text={}",
            row_index,
            bg_runs
                .iter()
                .map(|run| format!(
                    "{}:{}-{}",
                    terminal_rgb_hex(run.color),
                    run.start_col,
                    run.end_col
                ))
                .collect::<Vec<_>>()
                .join(","),
            text
        ));
    }

    if lines.is_empty() {
        "style rows: none".to_owned()
    } else {
        format!("style rows:\n{}", lines.join("\n"))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalReplayBgRun {
    color: TerminalRgb,
    start_col: usize,
    end_col: usize,
}

fn terminal_replay_bg_runs(
    row: &octty_term::live::TerminalRowSnapshot,
    default_bg: TerminalRgb,
) -> Vec<TerminalReplayBgRun> {
    let mut runs = Vec::new();
    let mut active: Option<TerminalReplayBgRun> = None;
    for (col, cell) in row.cells.iter().enumerate() {
        let color = cell.bg.unwrap_or(default_bg);
        match active.as_mut() {
            Some(run) if run.color == color => run.end_col = col + 1,
            Some(_) => {
                runs.push(active.take().expect("checked"));
                active = Some(TerminalReplayBgRun {
                    color,
                    start_col: col,
                    end_col: col + 1,
                });
            }
            None => {
                active = Some(TerminalReplayBgRun {
                    color,
                    start_col: col,
                    end_col: col + 1,
                });
            }
        }
    }
    if let Some(run) = active {
        runs.push(run);
    }
    runs
}

fn terminal_rgb_hex(color: TerminalRgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}
