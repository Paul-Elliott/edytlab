//! The files the app ships with: the templates, the bundled skills and the
//! third-party notices (`bundle.resources` in `tauri.conf.json`).
//!
//! On a desktop the installer puts them in a folder, Tauri's
//! `resource_dir()`, and the commands read them from there with `std::fs`.
//! An Android package holds them as assets instead, which have no path to
//! open and no folder to list. So the Android build carries the same files
//! inside the library (`build.rs` writes the list) and, once per launch,
//! makes sure they are in the app's own storage, under
//! `resources/<app version>/`, in the layout the installers use. The
//! commands then read that folder as they read `resource_dir()`.
//!
//! [`root`] is the one answer to "where are the resources", on every
//! platform.

use std::path::PathBuf;

use tauri::Manager;

#[cfg(any(target_os = "android", test))]
include!(concat!(env!("OUT_DIR"), "/bundled_resources.rs"));

/// The folder the resources are read from: `resource_dir()` on a desktop,
/// the extracted copy in the app's storage on Android.
pub fn root<R: tauri::Runtime, M: Manager<R>>(app: &M) -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        android_root(app)
    }
    #[cfg(not(target_os = "android"))]
    {
        app.path().resource_dir().map_err(|e| e.to_string())
    }
}

/// Where this version's resources go in the app's storage.
#[cfg(target_os = "android")]
fn android_root<R: tauri::Runtime, M: Manager<R>>(app: &M) -> Result<PathBuf, String> {
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(data
        .join("resources")
        .join(app.package_info().version.to_string()))
}

/// Write the resources to the app's storage if they are not there yet, and
/// remove what an earlier version of the app left. Called once from `setup`,
/// before any command can ask for a resource. A failure is logged: the app
/// still starts, without templates or bundled skills.
#[cfg(target_os = "android")]
pub fn prepare<R: tauri::Runtime, M: Manager<R>>(app: &M) {
    let root = match android_root(app) {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(%error, "no app data folder: the bundled resources are unavailable");
            return;
        }
    };
    if let Err(error) = extract_to(&root) {
        tracing::warn!(%error, dir = %root.display(), "could not unpack the bundled resources");
        return;
    }
    if let Some(parent) = root.parent() {
        remove_other_versions(parent, &root);
    }
}

/// Write every bundled file under `root`, skipping one that is already
/// there with the right length. Each file is written beside its name and
/// renamed into place, so a launch that is killed half way leaves either
/// the whole file or none.
#[cfg(any(target_os = "android", test))]
fn extract_to(root: &std::path::Path) -> std::io::Result<()> {
    for (relative, bytes) in BUNDLED_RESOURCES {
        let dest = root.join(relative);
        let current = std::fs::metadata(&dest).map(|m| m.len()).ok();
        if current == Some(bytes.len() as u64) {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = dest.with_extension("part");
        std::fs::write(&part, bytes)?;
        std::fs::rename(&part, &dest)?;
    }
    Ok(())
}

/// Remove every folder in `parent` but `keep`: the resources of versions
/// that are no longer installed.
#[cfg(any(target_os = "android", test))]
fn remove_other_versions(parent: &std::path::Path, keep: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != keep && path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded list is what `bundle.resources` ships: the templates,
    /// the skills and the notices, each under the folder the installers use.
    #[test]
    fn the_embedded_list_matches_the_resource_folders() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let count = |dir: &str| std::fs::read_dir(manifest.join(dir)).unwrap().count();
        let under = |prefix: &str| {
            BUNDLED_RESOURCES
                .iter()
                .filter(|(path, _)| path.starts_with(prefix))
                .count()
        };
        assert_eq!(under("templates/"), count("resources/templates"));
        assert_eq!(under("bundled-skills/"), count("resources/skills"));
        assert_eq!(under("licenses/"), 1);
        assert!(under("templates/") > 0 && under("bundled-skills/") > 0);
        assert!(BUNDLED_RESOURCES.iter().all(|(_, bytes)| !bytes.is_empty()));
    }

    #[test]
    fn extracting_writes_every_file_and_repairs_a_damaged_one() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("resources").join("1.2.3");
        extract_to(&root).unwrap();
        for (relative, bytes) in BUNDLED_RESOURCES {
            assert_eq!(&std::fs::read(root.join(relative)).unwrap(), bytes);
        }

        // A file cut short (the app was killed mid-write by an older
        // build, say) is written again; the others are left alone.
        let (first, bytes) = BUNDLED_RESOURCES[0];
        std::fs::write(root.join(first), b"x").unwrap();
        extract_to(&root).unwrap();
        assert_eq!(&std::fs::read(root.join(first)).unwrap(), bytes);
        // Nothing is left beside the files.
        let stray = std::fs::read_dir(root.join("templates"))
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "part"));
        assert!(!stray);
    }

    #[test]
    fn older_versions_are_removed_and_this_one_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("resources");
        let old = parent.join("0.9.3");
        let new = parent.join("0.9.4");
        std::fs::create_dir_all(old.join("templates")).unwrap();
        extract_to(&new).unwrap();
        remove_other_versions(&parent, &new);
        assert!(!old.exists());
        assert!(new.join("licenses/THIRD_PARTY_NOTICES.txt").is_file());
    }
}
