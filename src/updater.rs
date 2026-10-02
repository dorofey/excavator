//! Optional bundled Sparkle bridge. No update credentials live in the app.
//! Call from the GPUI main thread; Sparkle owns downloading, verification,
//! replacement, and relaunch UI.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, CString};
    use std::sync::OnceLock;

    struct Bridge {
        init: unsafe extern "C" fn() -> i32,
        check: unsafe extern "C" fn() -> i32,
        error: unsafe extern "C" fn() -> *const libc::c_char,
    }

    static BRIDGE: OnceLock<Result<Bridge, String>> = OnceLock::new();

    unsafe fn load() -> Result<Bridge, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let contents = executable
            .parent()
            .and_then(|path| path.parent())
            .ok_or("Could not locate the application bundle")?;
        let library = contents.join("Frameworks/ExcavatorUpdater.dylib");
        if !library.is_file() {
            return Err("Updates are available only in a packaged Excavator application.".into());
        }
        let path = CString::new(library.as_os_str().as_encoded_bytes())
            .map_err(|_| "Invalid updater library path")?;
        // Keep the handle open for the process lifetime: the controller and its
        // methods are Objective-C objects implemented by this library.
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            // Do not include native loader details, which may expose local paths.
            return Err("The bundled updater could not be loaded.".into());
        }
        let init = unsafe { libc::dlsym(handle, c"excavator_updater_init".as_ptr()) };
        let check = unsafe { libc::dlsym(handle, c"excavator_updater_check".as_ptr()) };
        let error = unsafe { libc::dlsym(handle, c"excavator_updater_error".as_ptr()) };
        if init.is_null() || check.is_null() || error.is_null() {
            return Err("The bundled updater has incompatible entry points.".into());
        }
        Ok(Bridge {
            init: unsafe {
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn() -> i32>(init)
            },
            check: unsafe {
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn() -> i32>(check)
            },
            error: unsafe {
                std::mem::transmute::<
                    *mut libc::c_void,
                    unsafe extern "C" fn() -> *const libc::c_char,
                >(error)
            },
        })
    }

    pub fn invoke(check: bool) -> Result<(), String> {
        let bridge = BRIDGE
            .get_or_init(|| unsafe { load() })
            .as_ref()
            .map_err(Clone::clone)?;
        let result = unsafe {
            if check {
                (bridge.check)()
            } else {
                (bridge.init)()
            }
        };
        if result != 0 {
            return Ok(());
        }
        let error = unsafe { (bridge.error)() };
        Err(if error.is_null() {
            "The updater could not start.".into()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        })
    }
}

pub fn initialize() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::invoke(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("In-app updates are supported on macOS.".into())
    }
}

pub fn check_for_updates() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::invoke(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("In-app updates are supported on macOS.".into())
    }
}
