use libghostty_vt::{RenderState, Terminal, TerminalOptions};

pub struct GhosttyTerminal {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
}

impl GhosttyTerminal {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self, libghostty_vt::Error> {
        Ok(Self {
            terminal: Terminal::new(TerminalOptions {
                cols,
                rows,
                max_scrollback,
            })?,
            render_state: RenderState::new()?,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), libghostty_vt::Error> {
        self.terminal.vt_write(bytes);
        Ok(())
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<(), libghostty_vt::Error> {
        self.terminal.resize(cols, rows, cell_width, cell_height)
    }

    pub fn mark_rendered(&mut self) -> Result<(), libghostty_vt::Error> {
        let snapshot = self.render_state.update(&self.terminal)?;
        snapshot.set_dirty(libghostty_vt::render::Dirty::Clean)
    }
}
