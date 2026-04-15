use super::*;

const OSC_MAX_NOTIFICATION_BYTES: usize = 4096;
const OSC_ESCAPE: u8 = 0x1b;
const OSC_BEL: u8 = 0x07;

#[derive(Default)]
pub(crate) struct TerminalOscNotificationParser {
    state: TerminalOscParseState,
    data: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
enum TerminalOscParseState {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

impl TerminalOscNotificationParser {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<TerminalNotification> {
        let mut notifications = Vec::new();
        for &byte in bytes {
            if let Some(notification) = self.push_byte(byte) {
                notifications.push(notification);
            }
        }
        notifications
    }

    fn push_byte(&mut self, byte: u8) -> Option<TerminalNotification> {
        match self.state {
            TerminalOscParseState::Ground => {
                if byte == OSC_ESCAPE {
                    self.state = TerminalOscParseState::Escape;
                }
            }
            TerminalOscParseState::Escape => {
                if byte == b']' {
                    self.data.clear();
                    self.state = TerminalOscParseState::Osc;
                } else if byte != OSC_ESCAPE {
                    self.state = TerminalOscParseState::Ground;
                }
            }
            TerminalOscParseState::Osc => match byte {
                OSC_BEL => return self.finish_sequence(),
                OSC_ESCAPE => self.state = TerminalOscParseState::OscEscape,
                _ => self.push_osc_byte(byte),
            },
            TerminalOscParseState::OscEscape => {
                if byte == b'\\' {
                    return self.finish_sequence();
                }
                self.push_osc_byte(OSC_ESCAPE);
                if byte == OSC_ESCAPE {
                    self.state = TerminalOscParseState::OscEscape;
                } else {
                    self.push_osc_byte(byte);
                    self.state = TerminalOscParseState::Osc;
                }
            }
        }
        None
    }

    fn push_osc_byte(&mut self, byte: u8) {
        if self.data.len() >= OSC_MAX_NOTIFICATION_BYTES {
            self.reset();
            return;
        }
        self.data.push(byte);
    }

    fn finish_sequence(&mut self) -> Option<TerminalNotification> {
        self.state = TerminalOscParseState::Ground;
        parse_terminal_osc_notification(&std::mem::take(&mut self.data))
    }

    fn reset(&mut self) {
        self.state = TerminalOscParseState::Ground;
        self.data.clear();
    }
}

pub(crate) fn parse_terminal_osc_notification(data: &[u8]) -> Option<TerminalNotification> {
    let command = String::from_utf8_lossy(data);

    if let Some(message) = command.strip_prefix("9;") {
        if message == "4" || message.starts_with("4;") {
            return None;
        }
        return terminal_notification_from_parts("Terminal", message);
    }

    let mut parts = command.splitn(4, ';');
    let code = parts.next()?;
    let action = parts.next()?;
    if code == "777" && action == "notify" {
        let title = parts.next().unwrap_or("Terminal");
        let body = parts.next().unwrap_or_default();
        return terminal_notification_from_parts(title, body);
    }

    None
}

fn terminal_notification_from_parts(title: &str, body: &str) -> Option<TerminalNotification> {
    let title = sanitize_terminal_notification_text(title);
    let body = sanitize_terminal_notification_text(body);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(TerminalNotification {
        title: if title.is_empty() {
            "Terminal".to_owned()
        } else {
            title
        },
        body,
    })
}

fn sanitize_terminal_notification_text(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_owned()
}
