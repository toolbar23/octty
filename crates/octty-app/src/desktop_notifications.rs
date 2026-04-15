use super::*;

const NOTIFICATION_TITLE_LIMIT: usize = 120;
const NOTIFICATION_BODY_LIMIT: usize = 600;

pub(crate) fn show_desktop_notification(notification: &TerminalNotification) {
    let title = notification_text_for_desktop(&notification.title, NOTIFICATION_TITLE_LIMIT);
    let body = notification_text_for_desktop(&notification.body, NOTIFICATION_BODY_LIMIT);
    std::thread::spawn(move || {
        if let Err(error) = show_desktop_notification_sync(&title, &body) {
            eprintln!("[octty-app] failed to show desktop notification: {error}");
        }
    });
}

fn notification_text_for_desktop(text: &str, limit: usize) -> String {
    let mut output = String::new();
    for character in text
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    {
        if output.chars().count() >= limit {
            output.push_str("...");
            break;
        }
        output.push(character);
    }
    output.trim().to_owned()
}

#[cfg(target_os = "linux")]
fn show_desktop_notification_sync(title: &str, body: &str) -> std::io::Result<()> {
    let status = std::process::Command::new("notify-send")
        .arg("--app-name=Octty")
        .arg("--")
        .arg(title)
        .arg(body)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "notify-send exited with {status}"
        )))
    }
}

#[cfg(target_os = "macos")]
fn show_desktop_notification_sync(title: &str, body: &str) -> std::io::Result<()> {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        applescript_escape(body),
        applescript_escape(title)
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "osascript exited with {status}"
        )))
    }
}

#[cfg(target_os = "macos")]
fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn show_desktop_notification_sync(_title: &str, _body: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "desktop notifications are not implemented for this platform",
    ))
}
