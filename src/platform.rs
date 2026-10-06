//! Platform launch integration; invoke only after explicit user action, in background.
use crate::domain::Location;

#[cfg(target_os = "macos")]
pub fn open_in_default_app(location: &Location) -> Result<(), String> {
    let result = std::process::Command::new("/usr/bin/open")
        .arg("--")
        .arg(location.local_path())
        .output()
        .map_err(|error| format!("Cannot open file: {error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Cannot open file: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ))
    }
}

#[cfg(target_os = "linux")]
pub fn open_in_default_app(location: &Location) -> Result<(), String> {
    let result = std::process::Command::new("xdg-open")
        .arg(location.local_path())
        .output()
        .map_err(|error| format!("Cannot open file: {error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Cannot open file: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn open_in_default_app(_location: &Location) -> Result<(), String> {
    Err("Opening files is not implemented on this platform".into())
}
