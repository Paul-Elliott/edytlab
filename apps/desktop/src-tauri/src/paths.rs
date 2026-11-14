//! Where the app's own folders are, on each platform.
//!
//! The desktop app keeps what belongs to no single project in `~/.edytlab`.
//! Android has no home folder an app may write to: Tauri's `home_dir()`
//! answers the shared storage root there, which needs a permission the app
//! does not ask for. So on Android the same `.edytlab` folder sits in the
//! app's private data folder, and every caller asks [`home_base`] instead of
//! `home_dir()`.

use std::path::PathBuf;
use tauri::Manager;

/// The folder `.edytlab` is created in: the home folder on a desktop, the
/// app's private data folder on Android. `None` when the platform cannot
/// say, in which case callers fall back to the app data folder as before.
pub fn home_base<R: tauri::Runtime, M: Manager<R>>(app: &M) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        app.path().app_data_dir().ok()
    }
    #[cfg(not(target_os = "android"))]
    {
        app.path().home_dir().ok()
    }
}
