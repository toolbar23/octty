use super::*;

pub(crate) fn terminal_rgb(color: RgbColor) -> TerminalRgb {
    TerminalRgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

pub(crate) fn renderer_error(error: libghostty_vt::Error) -> TerminalError {
    TerminalError::Renderer(error.to_string())
}

pub(crate) fn renderer_context(
    context: &'static str,
) -> impl FnOnce(libghostty_vt::Error) -> TerminalError {
    move |error| TerminalError::Renderer(format!("{context}: {error}"))
}

pub(crate) fn micros_since(start: Instant, end: Instant) -> u64 {
    end.saturating_duration_since(start)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}
