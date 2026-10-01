//! Blocking macOS Keychain adapter. Call only on a background executor.
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

const SERVICE: &str = "com.excavator.connection-secrets.v1";
const ITEM_NOT_FOUND: i32 = -25300;

pub fn get(id: &str) -> Result<Option<Vec<u8>>, String> {
    match get_generic_password(SERVICE, id) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(format!(
            "Cannot read connection credentials from Keychain (status {})",
            error.code()
        )),
    }
}

pub fn set(id: &str, bytes: &[u8]) -> Result<(), String> {
    set_generic_password(SERVICE, id, bytes).map_err(|error| {
        format!(
            "Cannot save connection credentials in Keychain (status {})",
            error.code()
        )
    })
}

pub fn remove(id: &str) -> Result<(), String> {
    match delete_generic_password(SERVICE, id) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(format!(
            "Cannot remove connection credentials from Keychain (status {})",
            error.code()
        )),
    }
}
