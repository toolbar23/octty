# Terminal Render Plan

Octty is currently much closer than before, but it still does not feel like
Ghostty because the render architecture is still shaped like a UI widget that
repaints terminal text, not like a terminal renderer.

The current pipeline is:

1. A real PTY runs `tmux`.
2. PTY output bytes are fed into `libghostty-vt`.
3. The VT state is extracted into an Octty terminal snapshot.
4. GPUI builds paint input from the snapshot.
5. GPUI shapes visible glyphs through a cache.
6. GPUI paints one background rectangle per background run and one glyph sprite
   per visible non-space terminal cell.

That model removed the worst mistakes from the first Rust version: the UI no
longer uses `tmux capture-pane` as the display source, shaping is cached, and
background runs are not split just because foreground syntax colors change.
The remaining gap is mostly step 6.

## The Gap

Real terminal emulators optimize around damage, GPU batches, and persistent
render state. They do not treat every terminal frame as a fresh collection of
general-purpose UI paint calls.

Octty still pays costs in places where a terminal should usually pay almost
nothing:

1. It extracts and considers a full visible grid for every frame.
2. It builds a fresh GPUI canvas surface for the whole terminal.
3. It inserts one GPUI monochrome sprite primitive per visible glyph.
4. It repaints unchanged rows because GPUI does not know terminal row damage.
5. It uses GPUI's generic text/glyph paint path rather than a terminal-specific
   instance buffer or row cache.

The current profiles show the shape clearly:

1. `render build` is usually sub-millisecond.
2. `shape` is low after warmup because the glyph cache hits.
3. `paint` grows with `glyph cells` and still spikes when dense editor views
   repaint many visible characters.

So the bottleneck is no longer "we shape too much text". It is "we submit too
many paint primitives through a renderer that is not specialized for terminal
grids".

## How Real Terminals Differ

Alacritty builds renderable cells from the terminal model, skips empty cells and
wide-character spacer cells early, uses a glyph cache, and fills renderer batches
instead of asking a UI toolkit to paint each text cell as an element.

WezTerm renders by screen line, caches shaped line elements, keeps glyphs in an
atlas, and turns screen-line data into GPU vertices. Dense text still means many
glyphs, but the submission path is batch-oriented.

Ghostty keeps renderer state separate from terminal state, has explicit render
and draw wakeups, uses row-oriented render data, and is designed so repeated
frames do not rebuild or repaint more than the changed terminal content.

Kitty uses sprite maps, text caches, and terminal cell shaders. The terminal
grid becomes indexed GPU work, not thousands of independent UI paint calls.

The common pattern is:

1. VT parser/model owns terminal truth.
2. Damage tracking decides which rows or cells changed.
3. Render extraction is incremental.
4. Glyph rasterization is cached.
5. Draw submission is batched.
6. Unchanged rows are reused.

Octty has 1 and parts of 4. It is missing 2, 3, 5, and 6 in the render path.

## Target Budgets

These are practical budgets for a natural-feeling terminal in a 60-120 Hz UI:

1. PTY read and command handoff: below 1 ms in normal cases.
2. VT parse/write: below 1-2 ms for normal editor updates.
3. Render-state update and snapshot extraction: below 2 ms for changed rows.
4. Paint-input construction: below 1 ms.
5. Text shaping: near zero on cache hits, only changed text should miss.
6. Paint submission: below 1-2 ms for normal editor interactions.
7. Dense full-screen repaint: allowed to be more expensive, but must be batched
   enough to avoid 20-70 ms paint spikes.

The key rule is that normal typing and cursor movement must scale with changed
rows or changed cells, not with the full visible grid.

## Step 1: Keep Dirty Rows From The VT Boundary

The VT integration must tell the app which rows changed when bytes are written.
If `libghostty-vt` exposes damage or dirty regions, use that directly. If it
does not, Octty should compute row hashes immediately after VT writes and keep a
small dirty-row set.

Output of this step:

- [x] Each terminal update includes changed row indices.
- [ ] Full-grid extraction is reserved for resize, reset, alternate-screen switch,
   scrollback changes, or damage uncertainty.
- [x] The profile reports dirty row count and dirty cell count.

A good first target is that holding a key in a shell marks one row dirty, not the
whole viewport.

## Step 2: Replace Full Snapshots With Render Frames

The UI should not receive a fresh standalone copy of every visible cell as the
primary render input. It should receive a render frame that references persistent
terminal render state.

The persistent state should keep:

- [x] Row identity or generation.
- [x] Cell text, colors, style, width, and flags.
- [x] Precomputed row hash.
- [x] Precomputed background runs.
- [x] Precomputed foreground glyph cells or glyph clusters.

Output of this step:

- [x] Unchanged rows reuse previous row render data.
- [x] `render build` only rebuilds dirty rows.
- [x] The profile reports rebuilt rows vs reused rows.

This changes the app from "snapshot to paint input every frame" to "dirty rows
update persistent render rows".

## Step 3: Cache Rows, Not Just Glyphs

The current glyph cache avoids repeated shaping, but a dense screen still loops
over every visible glyph and calls `paint_glyph` for each one. Row-level caching
should sit above the glyph cache.

Each render row should cache:

- [x] Background runs.
- [x] Foreground runs or glyph cells.
- [x] Shaped glyph references.
- [ ] A compact paint representation.

If the row generation is unchanged, the renderer should reuse the row paint
representation without rebuilding it.

Output of this step:

- [x] `shape` remains low.
- [x] `render build` remains low even on dense screens.
- [x] The paint path can distinguish reused rows from repainted rows.

This still does not fully solve GPUI paint submission, but it makes the remaining
cost visible and bounded.

## Step 4: Stop Painting Unchanged Rows Through GPUI

Caching row data is not enough if every frame still inserts glyph sprites for
every visible row. The next boundary is scene or surface reuse.

There are two viable approaches:

- [x] GPUI row scene reuse.
- [ ] A dedicated terminal render surface.

For GPUI row scene reuse, each terminal row must become a cacheable paint unit.
When a row generation does not change, GPUI should replay or retain the previous
row scene instead of rebuilding its glyph primitives.

For a dedicated terminal surface, Octty owns a terminal render target and updates
only dirty rows in that target. GPUI then composites the terminal surface as one
surface primitive.

Output of this step:

- [ ] Paint cost scales with dirty rows.
- [x] Holding a key in a shell repaints one row and cursor state, not the whole
   terminal.
- [x] Dense unchanged previews do not keep submitting thousands of glyph sprites.

This is the largest architectural gap between the current app and Ghostty.

## Step 5: Batch Glyph Submission

If GPUI row scene reuse is not enough, Octty needs a renderer path that submits
terminal glyphs as batches.

The batch model should be:

- [ ] Glyph atlas contains rasterized glyphs.
- [ ] A row or screen produces glyph instances.
- [ ] Instances contain glyph atlas id, position, color, and flags.
- [ ] Backgrounds are merged into rectangle instances.
- [ ] Dirty rows update a slice of instance data.
- [ ] The GPU draws all foreground glyphs in a few draw calls.

This is how the app gets from "one method call per visible glyph per frame" to
"one or a few submissions for the terminal".

Output of this step:

- [ ] GPUI no longer sees thousands of per-glyph paint calls for a terminal
      frame.
- [ ] Paint p95 for dense editor updates drops into the low milliseconds.
- [ ] CPU usage while running `btop` stops being dominated by UI paint
      submission.

## Step 6: Treat Cursor And Selection As Overlays

Cursor blink, cursor movement, and selection should not invalidate full terminal
rows unless cell contents actually changed.

Cursor and selection should be rendered as overlays:

- [x] Cursor background/foreground override is a small overlay.
- [ ] Selection rectangles are merged overlay runs.
- [ ] Cursor blink invalidates only the cursor cell.
- [x] Focus changes invalidate border and cursor style, not terminal contents.

Output of this step:

- [x] Blinking cursor does not repaint the full grid.
- [x] Focus changes do not rebuild terminal rows.
- [ ] Selection rendering is independent from VT row cache.

## Step 7: Keep Scheduling Separate From Rendering

Terminal IO and UI rendering should have separate clocks.

The app should:

- [x] Drain PTY output aggressively.
- [x] Update VT state immediately.
- [x] Coalesce UI wakeups to the display cadence.
- [x] Bypass coalescing for focused input when it affects latency.
- [x] Avoid creating runtimes or blocking handles in hot paths.

Output of this step:

- [x] Key-to-PTY latency remains low.
- [ ] UI paints at most once per frame.
- [x] Heavy output does not starve input.
- [x] Background terminals remain rate-limited.

This scheduling layer matters, but it cannot compensate for a paint path that
submits too many primitives.

## Step 8: Make The Profile Enforce The Model

The profile should show whether Octty is behaving like a terminal renderer.

Add these counters:

- [x] Dirty rows.
- [x] Rebuilt rows.
- [x] Reused rows.
- [x] Dirty cells.
- [x] Reused glyph layouts.
- [x] Submitted glyph primitives.
- [x] Submitted background primitives.
- [x] Replayed row scenes or updated surface rows.
- [x] Paint submission time.

The nvim picker-preview testcase should fail or warn when:

- [x] A one-row input update rebuilds most rows.
- [x] An unchanged dense preview submits all glyphs again.
- [ ] Paint p95 exceeds the target budget.
- [x] Background primitive count grows with foreground syntax runs.

The profile should make regressions obvious before the app feels bad.

## Execution Order

- [x] Add dirty-row tracking at the VT/render-state boundary.
- [x] Introduce persistent terminal render rows.
- [x] Rebuild render rows only when dirty.
- [x] Add row-level profiling.
- [x] Prototype GPUI row scene reuse.
- [ ] Measure whether scene reuse is enough.
- [ ] If not enough, build a dedicated batched terminal surface.
- [ ] Move cursor and selection into overlays.
- [x] Keep focused input scheduling unthrottled while preserving frame
      coalescing.
- [x] Use the picker-preview profile as the regression gate.

The important decision point is step 6. If GPUI can cheaply replay unchanged row
scenes, Octty can stay mostly inside GPUI. If it cannot, the terminal needs a
custom batched renderer and GPUI should only host/composite it.

## Success Criteria

The terminal feels natural when:

- [ ] Holding a key in a shell has no visible cadence or bursts.
- [ ] nvim picker preview remains interactive while previews update.
- [ ] `btop` does not drive Octty CPU far above a native terminal.
- [ ] Paint p95 stays low on dense screens after warmup.
- [ ] Cursor blink and focus changes do not move the full terminal pipeline.
- [ ] Profiles show costs proportional to damage, not viewport size.

Until those are true, small local optimizations can improve numbers, but they do
not close the architectural gap.
