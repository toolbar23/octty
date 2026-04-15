# Terminal Performance Notes

The picker-preview workload is a better proxy for editor latency than holding a
single printable key. It changes many lines, uses many style runs, and asks the
terminal to update dense text while the user is still interacting.

## What Real Terminals Optimize

Alacritty keeps the terminal model separate from rendering and turns it into an
iterator of renderable cells. Empty background cells and wide-character spacer
cells are skipped before the renderer sees them. Text drawing uses a glyph cache
and batches cells by atlas texture.

WezTerm renders by screen line, caches shaped line elements when it has a shape
key, and then fills GPU vertex buffers from the shaped result. Its WebGPU path
submits batched layers instead of creating one UI element per terminal cell.

Ghostty has a dedicated renderer thread with an 8 ms draw interval, separate
render and draw wakeups, and benchmarks the `RenderState` path because cloning or
rebuilding screen state is a primary IO/render lock holder.

The shared shape is:

1. PTY reads feed a VT parser/model.
2. The model tracks dirty state or renderable regions.
3. Render-state extraction is incremental or cache-friendly.
4. Glyph shaping/rasterization is cached by line/cluster/glyph.
5. Painting is GPU-batched, usually as quads/vertices, not as many retained UI
   elements.

## Local Budget

At 60 Hz, the whole UI frame budget is 16.7 ms. At 120 Hz, it is 8.3 ms. A
terminal should leave most of that budget for the app and compositor. For a
120x40 picker-preview frame, the target budget for our side should be roughly:

- PTY read and command handoff: sub-millisecond except occasional scheduler
  spikes.
- VT write/parse: under 1-2 ms for normal editor updates.
- Render-state update and snapshot extraction: under 2-3 ms combined.
- App paint-input construction: under 1 ms.
- Text shaping: near zero for unchanged cached lines; a few ms only for truly
  changed dense lines.
- Paint submission: under 1-2 ms and proportional to changed/batched runs, not
  rows times columns.

Those are working budgets, not claims about another emulator's exact timings on
this machine. The important diagnostic is whether a phase scales with the whole
grid every frame.

## Instrumentation

Runtime profiling:

```bash
OCTTY_TERMINAL_PROFILE=1 cargo run -p octty-app --bin octty-rs
```

The terminal label and stderr now include these buckets:

- `key`: app key event to live snapshot arrival.
- `pty`: PTY output command arrival to snapshot start.
- `vt`: accumulated `libghostty-vt` write time for output included in the
  snapshot.
- `upd`: `RenderState::update`.
- `extract`: Octty row/cell snapshot extraction.
- `snap`: full snapshot build.
- `dirty rows` / `dirty cells`: row-hash damage detected at the VT boundary.
- `render build`: Octty snapshot-to-persistent-row-cache construction.
- `rebuilt rows` / `reused rows`: app render rows rebuilt from dirty rows vs
  reused from the persistent row cache.
- `shape`: glyph-cache lookup time plus GPUI shaping for glyph cache misses
  when a row is repainted.
- `paint`: GPUI row paint time when a row is repainted.
- `glyph cells`: non-space terminal cells in the current render input.
- `glyph hits` / `glyph misses`: per-glyph layout cache reuse vs new shapes.

Synthetic profile tests:

```bash
cargo test -p octty-term --features ghostty-vt --lib picker_preview_vt_pipeline_profile -- --ignored --nocapture
cargo test -p octty-app --bin octty-rs terminal_picker_preview_paint_input_profile -- --ignored --nocapture
```

The first test exercises ANSI output into `libghostty-vt` and snapshot
extraction. The second exercises Octty's snapshot-to-paint-input path with a
dense nvim-style picker and preview.

## Sources

- Alacritty renderable content:
  https://github.com/alacritty/alacritty/blob/master/alacritty/src/display/content.rs
- Alacritty text renderer and glyph cache path:
  https://github.com/alacritty/alacritty/blob/master/alacritty/src/renderer/text/mod.rs
- WezTerm screen-line renderer:
  https://github.com/wez/wezterm/blob/main/wezterm-gui/src/termwindow/render/screen_line.rs
- WezTerm draw path:
  https://github.com/wez/wezterm/blob/main/wezterm-gui/src/termwindow/render/draw.rs
- Ghostty renderer thread:
  https://github.com/ghostty-org/ghostty/blob/main/src/renderer/Thread.zig
- Ghostty render-state benchmark:
  https://github.com/ghostty-org/ghostty/blob/main/src/benchmark/ScreenClone.zig
