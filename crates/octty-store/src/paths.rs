use std::path::PathBuf;

pub fn default_store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OCTTY_RS_STATE_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local")
        .join("share")
        .join("octty-rs")
        .join("state.turso")
}
