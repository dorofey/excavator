//! macOS launch integration; invoke only after explicit user action, in background.
use crate::domain::Location;

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
