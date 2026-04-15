fn record_terminal_render_build_profile(input: &TerminalGridPaintInput, build_micros: u64) {
    if !terminal_render_profile_enabled() {
        return;
    }

    let sample = TerminalRenderProfileSample {
        build_micros,
        rows: input.rows,
        cols: input.cols,
        glyph_cells: input.glyph_cells.len(),
        background_runs: input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum(),
        text_bytes: input.glyph_cells.iter().map(|cell| cell.text.len()).sum(),
        dirty_rows: input.dirty_rows,
        dirty_cells: input.dirty_cells,
        rebuilt_rows: input.rebuilt_rows,
        reused_rows: input.reused_rows,
        repaint_backgrounds: input.repaint_backgrounds,
        ..TerminalRenderProfileSample::default()
    };
    record_terminal_render_profile(sample);
}

fn terminal_full_render_profile_sample(
    surface: &TerminalFullPaintSurface,
    build_micros: u64,
) -> TerminalRenderProfileSample {
    TerminalRenderProfileSample {
        build_micros,
        rows: surface.input.rows,
        cols: surface.input.cols,
        glyph_cells: surface.input.glyph_cells.len(),
        glyph_cache_hits: surface.glyph_cache_hits,
        glyph_cache_misses: surface.glyph_cache_misses,
        background_runs: surface
            .input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum(),
        text_bytes: surface
            .input
            .glyph_cells
            .iter()
            .map(|cell| cell.text.len())
            .sum(),
        dirty_rows: surface.input.dirty_rows,
        dirty_cells: surface.input.dirty_cells,
        rebuilt_rows: surface.input.rebuilt_rows,
        reused_rows: surface.input.reused_rows,
        repaint_backgrounds: surface
            .input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum::<usize>()
            + 1,
        ..TerminalRenderProfileSample::default()
    }
}

fn record_terminal_render_profile(sample: TerminalRenderProfileSample) {
    if !terminal_render_profile_enabled() {
        return;
    }

    let profiler =
        TERMINAL_RENDER_PROFILER.get_or_init(|| Mutex::new(TerminalRenderProfiler::default()));
    let Ok(mut profiler) = profiler.lock() else {
        return;
    };
    profiler.record(sample);
    profiler.maybe_report(sample);
}

fn record_terminal_row_paint_profile(
    surface: &TerminalRowPaintSurface,
    cols: u16,
    paint_micros: u64,
) {
    if !terminal_render_profile_enabled() {
        return;
    }

    let profiler =
        TERMINAL_RENDER_PROFILER.get_or_init(|| Mutex::new(TerminalRenderProfiler::default()));
    let Ok(mut profiler) = profiler.lock() else {
        return;
    };
    profiler.record(TerminalRenderProfileSample {
        paint_micros,
        rows: 1,
        cols,
        painted_rows: 1,
        submitted_glyphs: surface.glyph_cells.len(),
        submitted_backgrounds: terminal_row_background_submission_count(&surface.row_input),
        ..TerminalRenderProfileSample::default()
    });
}

fn terminal_render_profile_summary() -> Option<String> {
    if !terminal_render_profile_enabled() {
        return None;
    }

    let profiler = TERMINAL_RENDER_PROFILER.get()?;
    let profiler = profiler.lock().ok()?;
    profiler.summary()
}

fn terminal_render_profile_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| terminal_env_flag_enabled("OCTTY_TERMINAL_PROFILE"))
}

fn terminal_performance_data_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        terminal_env_flag_enabled("OCTTY_TERMINAL_PERF") || terminal_render_profile_enabled()
    })
}

fn terminal_env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| terminal_env_value_enabled(&value))
}

fn terminal_env_value_enabled(value: &str) -> bool {
    value != "0" && !value.eq_ignore_ascii_case("false")
}

static TERMINAL_RENDER_PROFILER: OnceLock<Mutex<TerminalRenderProfiler>> = OnceLock::new();

impl TerminalRenderProfiler {
    fn record(&mut self, sample: TerminalRenderProfileSample) {
        if sample.build_micros > 0 {
            push_latency_sample(&mut self.build_micros, sample.build_micros);
        }
        if sample.shape_micros > 0 {
            push_latency_sample(&mut self.shape_micros, sample.shape_micros);
        }
        let has_build_counts = sample.build_micros > 0
            || sample.dirty_rows > 0
            || sample.dirty_cells > 0
            || sample.rebuilt_rows > 0
            || sample.reused_rows > 0
            || sample.repaint_backgrounds > 0
            || sample.glyph_cells > 0
            || sample.background_runs > 0
            || sample.text_bytes > 0;
        if has_build_counts {
            push_latency_sample(&mut self.glyph_cells, sample.glyph_cells as u64);
            push_latency_sample(&mut self.background_runs, sample.background_runs as u64);
            push_latency_sample(&mut self.text_bytes, sample.text_bytes as u64);
            push_latency_sample(&mut self.dirty_rows, sample.dirty_rows as u64);
            push_latency_sample(&mut self.dirty_cells, sample.dirty_cells as u64);
            push_latency_sample(&mut self.rebuilt_rows, sample.rebuilt_rows as u64);
            push_latency_sample(&mut self.reused_rows, sample.reused_rows as u64);
            push_latency_sample(
                &mut self.repaint_backgrounds,
                sample.repaint_backgrounds as u64,
            );
        }
        let has_shape_counts =
            sample.shape_micros > 0 || sample.glyph_cache_hits > 0 || sample.glyph_cache_misses > 0;
        if has_shape_counts {
            push_latency_sample(&mut self.glyph_cache_hits, sample.glyph_cache_hits as u64);
            push_latency_sample(
                &mut self.glyph_cache_misses,
                sample.glyph_cache_misses as u64,
            );
        }
        let has_paint_counts = sample.paint_micros > 0
            || sample.painted_rows > 0
            || sample.submitted_glyphs > 0
            || sample.submitted_backgrounds > 0;
        if has_paint_counts {
            push_latency_sample(&mut self.paint_micros, sample.paint_micros);
            push_latency_sample(&mut self.painted_rows, sample.painted_rows as u64);
            push_latency_sample(&mut self.submitted_glyphs, sample.submitted_glyphs as u64);
            push_latency_sample(
                &mut self.submitted_backgrounds,
                sample.submitted_backgrounds as u64,
            );
        }
    }

    fn summary(&self) -> Option<String> {
        let build = latency_summary(&self.build_micros)?;
        let mut parts = vec![format!("render build {build}")];
        if let Some(shape) = latency_summary(&self.shape_micros) {
            parts.push(format!("shape {shape}"));
        }
        if let Some(paint) = latency_summary(&self.paint_micros) {
            parts.push(format!("row paint {paint}"));
        }
        Some(parts.join(" · "))
    }

    fn maybe_report(&mut self, sample: TerminalRenderProfileSample) {
        let now = Instant::now();
        if self
            .last_report_at
            .is_some_and(|reported_at| now.duration_since(reported_at) < Duration::from_secs(1))
        {
            return;
        }
        self.last_report_at = Some(now);

        let Some(summary) = self.summary() else {
            return;
        };
        eprintln!(
            "octty terminal render profile: {summary} · grid {}x{} · dirty rows {} · dirty cells {} · rebuilt rows {} · reused rows {} · repainted bgs {} · painted rows {} · glyph cells {} · submitted glyphs {} · glyph hits {} · glyph misses {} · bg runs {} · submitted bgs {} · text bytes {}",
            sample.cols,
            sample.rows,
            count_summary(&self.dirty_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.dirty_cells).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.rebuilt_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.reused_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.repaint_backgrounds).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.painted_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cells).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.submitted_glyphs).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cache_hits).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cache_misses).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.background_runs).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.submitted_backgrounds).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.text_bytes).unwrap_or_else(|| "n/a".to_owned())
        );
    }
}
