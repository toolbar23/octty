use super::*;

const OSC_MAX_NOTIFICATION_BYTES: usize = 4096;
const OSC_ESCAPE: u8 = 0x1b;
const OSC_BEL: u8 = 0x07;

#[derive(Default)]
pub(crate) struct TerminalOscNotificationParser {
    state: TerminalOscParseState,
    data: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct TerminalOscPassthroughFilter {
    state: TerminalOscParseState,
    data: Vec<u8>,
    sequence: Vec<u8>,
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

impl TerminalOscPassthroughFilter {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.push_byte(byte, &mut output);
        }
        output
    }

    fn push_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        match self.state {
            TerminalOscParseState::Ground => {
                if byte == OSC_ESCAPE {
                    self.sequence.clear();
                    self.sequence.push(byte);
                    self.state = TerminalOscParseState::Escape;
                } else {
                    output.push(byte);
                }
            }
            TerminalOscParseState::Escape => {
                self.sequence.push(byte);
                if byte == b']' {
                    self.data.clear();
                    self.state = TerminalOscParseState::Osc;
                } else {
                    output.extend_from_slice(&self.sequence);
                    self.reset();
                }
            }
            TerminalOscParseState::Osc => match byte {
                OSC_BEL => {
                    self.sequence.push(byte);
                    self.finish_sequence(output);
                }
                OSC_ESCAPE => {
                    self.sequence.push(byte);
                    self.state = TerminalOscParseState::OscEscape;
                }
                _ => self.push_osc_byte(byte, output),
            },
            TerminalOscParseState::OscEscape => {
                self.sequence.push(byte);
                if byte == b'\\' {
                    self.finish_sequence(output);
                } else {
                    if self.data.len().saturating_add(2) >= OSC_MAX_NOTIFICATION_BYTES {
                        output.extend_from_slice(&self.sequence);
                        self.reset();
                        return;
                    }
                    self.data.push(OSC_ESCAPE);
                    self.data.push(byte);
                    self.state = TerminalOscParseState::Osc;
                }
            }
        }
    }

    fn push_osc_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        if self.data.len() >= OSC_MAX_NOTIFICATION_BYTES {
            output.extend_from_slice(&self.sequence);
            self.reset();
            return;
        }
        self.sequence.push(byte);
        self.data.push(byte);
    }

    fn finish_sequence(&mut self, output: &mut Vec<u8>) {
        if !should_strip_terminal_osc_sequence(&self.data) {
            output.extend_from_slice(&self.sequence);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.state = TerminalOscParseState::Ground;
        self.data.clear();
        self.sequence.clear();
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

    if let Some(metadata) = terminal_command_end_metadata(&command) {
        return terminal_command_end_notification(metadata);
    }

    let mut parts = command.splitn(4, ';');
    let code = parts.next()?;
    let action = parts.next()?;
    if code == "777" && action == "notify" {
        let title = parts.next().unwrap_or("Terminal");
        let body = parts.next().unwrap_or_default();
        return terminal_notification_from_parts(title, body);
    }
    if is_shell_integration_command_end(code, action) {
        return None;
    }

    None
}

pub(crate) fn should_strip_terminal_osc_sequence(data: &[u8]) -> bool {
    let command = String::from_utf8_lossy(data);
    command.starts_with("133;") || command.starts_with("633;")
}

fn is_shell_integration_command_end(code: &str, action: &str) -> bool {
    matches!(code, "133" | "633") && action == "D"
}

fn terminal_command_end_metadata<'a>(command: &'a str) -> Option<&'a str> {
    match command {
        "133;D" | "633;D" => Some(""),
        _ => command
            .strip_prefix("133;D;")
            .or_else(|| command.strip_prefix("633;D;")),
    }
}

fn terminal_command_end_notification(metadata: &str) -> Option<TerminalNotification> {
    let metadata = metadata.trim();
    let fields = terminal_metadata_fields(metadata);
    if metadata.is_empty()
        || metadata == "0"
        || metadata.eq_ignore_ascii_case("success")
        || metadata.contains("exit=success")
        || command_exit_status(&fields).is_some_and(|status| status == "0")
    {
        return terminal_notification_from_parts("Command finished", "Command completed.");
    }

    let status =
        command_exit_status(&fields).or_else(|| metadata.parse::<i32>().ok().map(|_| metadata));
    let signal = fields.get("signal").copied();
    let body = match (status, signal) {
        (Some(status), Some(signal)) => format!("Exited with status {status} ({signal})."),
        (Some(status), None) => format!("Exited with status {status}."),
        (None, Some(signal)) => format!("Exited with signal {signal}."),
        (None, None) => "Command exited unsuccessfully.".to_owned(),
    };
    terminal_notification_from_parts("Command needs attention", &body)
}

fn command_exit_status<'a>(fields: &HashMap<&str, &'a str>) -> Option<&'a str> {
    fields
        .get("status")
        .copied()
        .or_else(|| fields.get("exit_code").copied())
        .or_else(|| fields.get("exitcode").copied())
}

fn terminal_metadata_fields(metadata: &str) -> HashMap<&str, &str> {
    metadata
        .split(';')
        .filter_map(|field| field.split_once('='))
        .collect()
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
