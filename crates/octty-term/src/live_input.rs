use super::*;

pub(crate) struct KeyInputEncoder<'alloc> {
    encoder: key::Encoder<'alloc>,
    event: key::Event<'alloc>,
}

impl<'alloc> KeyInputEncoder<'alloc> {
    pub(crate) fn new() -> Result<Self, TerminalError> {
        Ok(Self {
            encoder: key::Encoder::new().map_err(renderer_error)?,
            event: key::Event::new().map_err(renderer_error)?,
        })
    }

    pub(crate) fn encode(
        &mut self,
        terminal: &Terminal<'alloc, '_>,
        input: LiveTerminalKeyInput,
    ) -> Result<Vec<u8>, TerminalError> {
        let mut mods = key::Mods::empty();
        if input.modifiers.shift {
            mods |= key::Mods::SHIFT;
        }
        if input.modifiers.alt {
            mods |= key::Mods::ALT;
        }
        if input.modifiers.control {
            mods |= key::Mods::CTRL;
        }
        if input.modifiers.platform {
            mods |= key::Mods::SUPER;
        }

        let mut consumed_mods = key::Mods::empty();
        if input.text.is_some() && input.modifiers.shift {
            consumed_mods |= key::Mods::SHIFT;
        }

        self.event
            .set_action(key::Action::Press)
            .set_key(key_from_live_key(input.key))
            .set_mods(mods)
            .set_consumed_mods(consumed_mods)
            .set_unshifted_codepoint(input.unshifted)
            .set_utf8(input.text);

        let mut response = Vec::with_capacity(64);
        self.encoder
            .set_options_from_terminal(terminal)
            .encode_to_vec(&self.event, &mut response)
            .map_err(renderer_error)?;
        Ok(response)
    }
}

pub(crate) fn key_from_live_key(key: LiveTerminalKey) -> key::Key {
    match key {
        LiveTerminalKey::Character('a' | 'A') => key::Key::A,
        LiveTerminalKey::Character('b' | 'B') => key::Key::B,
        LiveTerminalKey::Character('c' | 'C') => key::Key::C,
        LiveTerminalKey::Character('d' | 'D') => key::Key::D,
        LiveTerminalKey::Character('e' | 'E') => key::Key::E,
        LiveTerminalKey::Character('f' | 'F') => key::Key::F,
        LiveTerminalKey::Character('g' | 'G') => key::Key::G,
        LiveTerminalKey::Character('h' | 'H') => key::Key::H,
        LiveTerminalKey::Character('i' | 'I') => key::Key::I,
        LiveTerminalKey::Character('j' | 'J') => key::Key::J,
        LiveTerminalKey::Character('k' | 'K') => key::Key::K,
        LiveTerminalKey::Character('l' | 'L') => key::Key::L,
        LiveTerminalKey::Character('m' | 'M') => key::Key::M,
        LiveTerminalKey::Character('n' | 'N') => key::Key::N,
        LiveTerminalKey::Character('o' | 'O') => key::Key::O,
        LiveTerminalKey::Character('p' | 'P') => key::Key::P,
        LiveTerminalKey::Character('q' | 'Q') => key::Key::Q,
        LiveTerminalKey::Character('r' | 'R') => key::Key::R,
        LiveTerminalKey::Character('s' | 'S') => key::Key::S,
        LiveTerminalKey::Character('t' | 'T') => key::Key::T,
        LiveTerminalKey::Character('u' | 'U') => key::Key::U,
        LiveTerminalKey::Character('v' | 'V') => key::Key::V,
        LiveTerminalKey::Character('w' | 'W') => key::Key::W,
        LiveTerminalKey::Character('x' | 'X') => key::Key::X,
        LiveTerminalKey::Character('y' | 'Y') => key::Key::Y,
        LiveTerminalKey::Character('z' | 'Z') => key::Key::Z,
        LiveTerminalKey::Character('0') => key::Key::Digit0,
        LiveTerminalKey::Character('1') => key::Key::Digit1,
        LiveTerminalKey::Character('2') => key::Key::Digit2,
        LiveTerminalKey::Character('3') => key::Key::Digit3,
        LiveTerminalKey::Character('4') => key::Key::Digit4,
        LiveTerminalKey::Character('5') => key::Key::Digit5,
        LiveTerminalKey::Character('6') => key::Key::Digit6,
        LiveTerminalKey::Character('7') => key::Key::Digit7,
        LiveTerminalKey::Character('8') => key::Key::Digit8,
        LiveTerminalKey::Character('9') => key::Key::Digit9,
        LiveTerminalKey::Character('-') => key::Key::Minus,
        LiveTerminalKey::Character('=') => key::Key::Equal,
        LiveTerminalKey::Character('[') => key::Key::BracketLeft,
        LiveTerminalKey::Character(']') => key::Key::BracketRight,
        LiveTerminalKey::Character('\\') => key::Key::Backslash,
        LiveTerminalKey::Character(';') => key::Key::Semicolon,
        LiveTerminalKey::Character('\'') => key::Key::Quote,
        LiveTerminalKey::Character(',') => key::Key::Comma,
        LiveTerminalKey::Character('.') => key::Key::Period,
        LiveTerminalKey::Character('/') => key::Key::Slash,
        LiveTerminalKey::Character('`') => key::Key::Backquote,
        LiveTerminalKey::Character(' ') | LiveTerminalKey::Space => key::Key::Space,
        LiveTerminalKey::Enter => key::Key::Enter,
        LiveTerminalKey::Backspace => key::Key::Backspace,
        LiveTerminalKey::Delete => key::Key::Delete,
        LiveTerminalKey::Tab => key::Key::Tab,
        LiveTerminalKey::Escape => key::Key::Escape,
        LiveTerminalKey::ArrowLeft => key::Key::ArrowLeft,
        LiveTerminalKey::ArrowRight => key::Key::ArrowRight,
        LiveTerminalKey::ArrowUp => key::Key::ArrowUp,
        LiveTerminalKey::ArrowDown => key::Key::ArrowDown,
        LiveTerminalKey::Home => key::Key::Home,
        LiveTerminalKey::End => key::Key::End,
        LiveTerminalKey::PageUp => key::Key::PageUp,
        LiveTerminalKey::PageDown => key::Key::PageDown,
        LiveTerminalKey::Insert => key::Key::Insert,
        LiveTerminalKey::F(1) => key::Key::F1,
        LiveTerminalKey::F(2) => key::Key::F2,
        LiveTerminalKey::F(3) => key::Key::F3,
        LiveTerminalKey::F(4) => key::Key::F4,
        LiveTerminalKey::F(5) => key::Key::F5,
        LiveTerminalKey::F(6) => key::Key::F6,
        LiveTerminalKey::F(7) => key::Key::F7,
        LiveTerminalKey::F(8) => key::Key::F8,
        LiveTerminalKey::F(9) => key::Key::F9,
        LiveTerminalKey::F(10) => key::Key::F10,
        LiveTerminalKey::F(11) => key::Key::F11,
        LiveTerminalKey::F(12) => key::Key::F12,
        LiveTerminalKey::F(_) | LiveTerminalKey::Character(_) => key::Key::Unidentified,
    }
}
