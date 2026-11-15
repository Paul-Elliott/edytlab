//! What the Android app does differently, in one place.
//!
//! The desktop app lets the person pick any folder as a project and any path
//! as an export, because the operating system has a file system they can
//! see. Android does not: the system picker hands back `content://`
//! addresses, there is no folder picker, and an app writes only inside its
//! own storage. So on Android:
//!
//! * **Projects** live in the app's storage, under `projects/`, and are
//!   chosen from a list ([`list_managed_projects`], [`create_managed_project`]).
//! * **Audio comes in** through the system picker and is copied into the
//!   open project's `imports/` folder, then loaded as any file is
//!   ([`import_audio`]).
//! * **Exports** are written to `exports/` in the app's storage and leave
//!   through the share sheet or a "save to" dialog ([`share_export`],
//!   [`save_export_to`]).
//! * **API keys** are kept in the app's storage, encrypted with a key in the
//!   Android Keystore (`KeystoreSecrets`), because the `keyring` crate the
//!   desktops use has no Android store and would forget them at each launch.
//!
//! The commands that only decide paths work on every platform, which is what
//! lets the host tests cover them. The ones that need Android itself (the
//! picker's files, the share sheet, the device's memory) go through
//! `EdytlabPlugin.kt`, registered by [`init`], and answer [`UNAVAILABLE`]
//! anywhere else. None of the Kotlin commands is granted to the webview.

use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::commands::{BatchLoadFailure, BatchLoadResult};
use crate::project::RecentProject;
use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

/// What a command that needs Android answers on a desktop.
pub const UNAVAILABLE: &str = "This is only available in the Android app.";

/// The folder inside a project that picked files are copied into.
pub const IMPORTS_DIR: &str = "imports";

/// The handle to `EdytlabPlugin.kt`.
#[cfg(target_os = "android")]
struct Bridge<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// The plugin that loads `EdytlabPlugin.kt`. Registered on Android only
/// (`lib.rs`). It defines no command of its own, so the webview can invoke
/// nothing through it.
#[cfg(target_os = "android")]
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::<R>::new("edytlab-android")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.edytlab.android", "EdytlabPlugin")?;
            app.manage(Bridge(handle));
            // Where the API keys are kept ([`KeystoreSecrets`]). Here, right
            // after the bridge it calls through and before the app's own
            // `setup` runs, so it is in place when the startup thread reads
            // the keys. Until it is, `ai::keychain` refuses every call.
            if !ai::keychain::install_platform_store(Box::new(KeystoreSecrets(app.clone()))) {
                tracing::warn!("a secret store was installed before the Android Keystore one");
            }
            Ok(())
        })
        .build()
}

/// Call a command of `EdytlabPlugin.kt` and wait for its answer. Blocks, so
/// it belongs on the blocking pool.
///
/// Not named `call`: `tests/main_thread_commands.rs` follows calls by name
/// to find what can wait on a session lock, and another `call` in this
/// crate does.
#[cfg(target_os = "android")]
fn ask_android<R: Runtime, T: DeserializeOwned>(
    app: &AppHandle<R>,
    command: &str,
    payload: impl Serialize,
) -> CmdResult<T> {
    use tauri::plugin::mobile::PluginInvokeError;

    let bridge = app
        .try_state::<Bridge<R>>()
        .ok_or_else(|| "the Android side of the app is not ready".to_string())?;
    bridge
        .0
        .run_mobile_plugin(command, payload)
        .map_err(|error| match error {
            // The Kotlin side's own words, without the wrapper's.
            PluginInvokeError::InvokeRejected(response) => response
                .message
                .unwrap_or_else(|| "the request was refused".to_string()),
            other => other.to_string(),
        })
}

#[cfg(not(target_os = "android"))]
fn ask_android<R: Runtime, T: DeserializeOwned>(
    _app: &AppHandle<R>,
    _command: &str,
    _payload: impl Serialize,
) -> CmdResult<T> {
    Err(UNAVAILABLE.to_string())
}

/// Run blocking work off the async workers and wait for it.
async fn off_thread<T, F>(work: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("the task did not finish: {e}"))?
}

// ---------------------------------------------------------------------------
// Secrets: the Android Keystore
// ---------------------------------------------------------------------------

/// Where `ai::keychain` keeps its secrets on Android (the API keys and the
/// ChatGPT sign-in's token, together the one `secrets` item): files in the
/// app's own storage, encrypted with a key that never leaves the Android
/// Keystore (`EdytlabPlugin.secretGet`, `secretSet` and `secretDelete`).
///
/// Every desktop uses the `keyring` crate, which has no Android store: built
/// for Android it keeps values in memory, and a saved key is gone at the next
/// launch. [`init`] installs this in its place.
///
/// Each call waits for Kotlin's answer, as [`ask_android`] does. Kotlin takes the
/// command on Android's main thread, so a call made from that thread would
/// wait on itself; the keychain is read from the startup thread, the
/// blocking pool and the commands' own threads.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct KeystoreSecrets<R: Runtime>(AppHandle<R>);

impl<R: Runtime> ai::keychain::SecretStore for KeystoreSecrets<R> {
    fn get(&self, account: &str) -> CmdResult<Option<Vec<u8>>> {
        let answer: serde_json::Value = ask_android(
            &self.0,
            "secretGet",
            serde_json::json!({ "account": account }),
        )?;
        secret_in(&answer)
    }

    fn set(&self, account: &str, value: &[u8]) -> CmdResult<()> {
        ask_android::<R, serde_json::Value>(
            &self.0,
            "secretSet",
            serde_json::json!({
                "account": account,
                "value": ai::keychain::secret_to_text(value),
            }),
        )?;
        Ok(())
    }

    fn delete(&self, account: &str) -> CmdResult<()> {
        ask_android::<R, serde_json::Value>(
            &self.0,
            "secretDelete",
            serde_json::json!({ "account": account }),
        )?;
        Ok(())
    }
}

/// The secret in what `EdytlabPlugin.secretGet` answered: `{ "value": … }`
/// with the bytes as base64, or with `null` when nothing is saved.
///
/// Read from loose JSON, not into a struct: serde's words for an answer that
/// does not fit quote the value it found, and those words end up in a log.
/// An answer without a `value` is an error. Only an explicit `null` may say
/// "nothing saved", because that tells the app the person has no key.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn secret_in(answer: &serde_json::Value) -> CmdResult<Option<Vec<u8>>> {
    match answer.get("value") {
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => ai::keychain::secret_from_text(text).map(Some),
        _ => Err("the Android Keystore's answer was not understood".to_string()),
    }
}

// ---------------------------------------------------------------------------
// The device
// ---------------------------------------------------------------------------

/// What the window needs to know about where it is running.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    /// True in the Android app.
    pub android: bool,
    /// The device's memory, when the platform says.
    pub total_ram_bytes: Option<u64>,
    /// "Google Pixel 8", for the Settings line. Android only.
    pub model: Option<String>,
    /// Whether stem separation is offered here. Always on a desktop; on
    /// Android only with the memory the model needs
    /// (`separate_stems::ANDROID_MIN_TOTAL_RAM_BYTES`), which the tool
    /// checks again before it loads the model.
    pub stems_supported: bool,
}

/// What `EdytlabPlugin.deviceInfo` answers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) struct NativeDeviceInfo {
    pub total_ram_bytes: u64,
    #[serde(default)]
    pub model: Option<String>,
}

/// Ask Android about the device. `Err` on a desktop.
pub(crate) fn native_device_info<R: Runtime>(app: &AppHandle<R>) -> CmdResult<NativeDeviceInfo> {
    ask_android(app, "deviceInfo", serde_json::json!({}))
}

/// Where the window is running and how much memory the device has.
#[tauri::command]
pub async fn device_info<R: Runtime>(app: AppHandle<R>) -> CmdResult<DeviceInfo> {
    if !cfg!(target_os = "android") {
        return Ok(DeviceInfo {
            android: false,
            total_ram_bytes: None,
            model: None,
            stems_supported: true,
        });
    }
    off_thread(move || {
        let native = native_device_info(&app).ok();
        let total_ram_bytes = native.as_ref().map(|n| n.total_ram_bytes);
        Ok(DeviceInfo {
            android: true,
            total_ram_bytes,
            model: native.and_then(|n| n.model),
            // A total that cannot be read hides nothing: the tool's own
            // check is the one that refuses.
            stems_supported: !total_ram_bytes
                .is_some_and(tools::tool::separate_stems::too_small_for_stems),
        })
    })
    .await
}

/// What is running that the screen must stay on for (an assistant turn, a
/// model download), one bit each. See [`set_busy`].
///
/// Bits in an atomic and not names in a set behind a mutex: this is touched
/// at the start of every turn, on the async runtime's worker, and nothing
/// there may wait on a lock (`tests/main_thread_commands.rs`).
static BUSY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The bit for `reason`. Two reasons can share one, in which case the first
/// of them to end lets the screen sleep while the other still runs; with a
/// turn and three model downloads over 64 bits that is not a case that
/// comes up, and it costs a dark screen, not work.
fn busy_bit(reason: &str) -> u64 {
    // FNV-1a: the same bit for the same name in every build.
    let hash = reason.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    1 << (hash % 64)
}

/// Mark `reason` as running or not. Answers the new state of the screen
/// when this call changed it: `Some(true)` when nothing was running and now
/// something is, `Some(false)` when the last thing has just ended.
fn note_busy(busy: &std::sync::atomic::AtomicU64, reason: &str, running: bool) -> Option<bool> {
    use std::sync::atomic::Ordering;
    let bit = busy_bit(reason);
    let (before, after) = if running {
        let before = busy.fetch_or(bit, Ordering::SeqCst);
        (before, before | bit)
    } else {
        let before = busy.fetch_and(!bit, Ordering::SeqCst);
        (before, before & !bit)
    };
    ((before == 0) != (after == 0)).then_some(after != 0)
}

/// Say that the long work called `reason` has started or ended. While any is
/// running the Android app keeps the screen on: Android stops an app's work
/// soon after its screen goes off, which would end a stem separation, a
/// transcription or a model download part way. A no-op on a desktop.
///
/// The call to Android is made on the blocking pool and its failure is only
/// logged, so the work never waits for it.
pub(crate) fn set_busy<R: Runtime>(app: &AppHandle<R>, reason: &str, running: bool) {
    if !cfg!(target_os = "android") {
        return;
    }
    let Some(on) = note_busy(&BUSY, reason, running) else {
        return;
    };
    let app = app.clone();
    // On the blocking pool, and not a thread started here by hand: the
    // scan in `tests/main_thread_commands.rs` follows calls by name, and a
    // plain thread start reads to it as the reclaimer's, which takes locks.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = ask_android::<R, serde_json::Value>(
            &app,
            "keepScreenOn",
            serde_json::json!({ "on": on }),
        ) {
            tracing::warn!(%error, on, "could not change keep-screen-on");
        }
    });
}

/// [`set_busy`] for as long as this is alive: the screen stays on from
/// `Busy::during` until the value is dropped, however the work ends.
pub(crate) struct Busy<R: Runtime> {
    app: AppHandle<R>,
    reason: &'static str,
}

impl<R: Runtime> Busy<R> {
    pub(crate) fn during(app: &AppHandle<R>, reason: &'static str) -> Self {
        set_busy(app, reason, true);
        Self {
            app: app.clone(),
            reason,
        }
    }
}

impl<R: Runtime> Drop for Busy<R> {
    fn drop(&mut self) {
        set_busy(&self.app, self.reason, false);
    }
}

// ---------------------------------------------------------------------------
// What the agent is told
// ---------------------------------------------------------------------------

/// The exports folder, once [`prepare_storage`] has made it. Set on Android
/// only, which is what makes [`with_platform_note`] a no-op on a desktop.
static EXPORTS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Make the exports folder and work from it. Called once from `setup` on
/// Android.
///
/// The working folder matters because several export tools take the path
/// the model gives them as it is: a bare file name would otherwise land in
/// the process's working folder, which on Android is `/` and cannot be
/// written to. A failure is logged; exports then fail with their own error.
#[cfg(target_os = "android")]
pub fn prepare_storage<R: Runtime, M: Manager<R>>(app: &M) {
    let dir = match exports_dir(app) {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(%error, "no exports folder");
            return;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(%error, dir = %dir.display(), "could not create the exports folder");
        return;
    }
    if let Err(error) = std::env::set_current_dir(&dir) {
        tracing::warn!(%error, "could not make the exports folder the working folder");
    }
    let _ = EXPORTS_DIR.set(dir);
}

/// What the model has to know about Android that the system prompt, written
/// for a desktop, does not say: there are no paths to ask the person for,
/// and one place exports go.
pub(crate) fn platform_note_for(exports: &Path) -> String {
    format!(
        "## Running on Android
         This is the Android app. The person cannot see the file system and cannot type or          choose a folder path, so never ask for one.
         - Exports: every tool that writes a file the person asked for (`render_final`,          `export_multiple`, `export_transcript`, a batch render, and any other tool with an          output path) must write inside `{}`. Give an absolute path in that folder, with a          short descriptive file name and the right extension. Choose the name yourself.
         - After an export, tell the person the file name and that it is in Exports, where          they can share it or save it to their device. Do not show them the folder path.
         - Loading audio: the person taps Import and picks files; those arrive as tracks on          their own. Do not ask for the path of a file. Files already imported are in the          project's `{IMPORTS_DIR}` folder.",
        exports.display()
    )
}

/// The active agent profile's text with the Android note in front of it.
/// Unchanged on a desktop.
pub(crate) fn with_platform_note(profile_body: Option<String>) -> Option<String> {
    let Some(exports) = EXPORTS_DIR.get() else {
        return profile_body;
    };
    let note = platform_note_for(exports);
    Some(match profile_body {
        Some(body) => format!("{note}\n\n{body}"),
        None => note,
    })
}

// ---------------------------------------------------------------------------
// Projects in the app's storage
// ---------------------------------------------------------------------------

/// `projects/` in the app's data folder.
fn projects_root<R: Runtime>(app: &AppHandle<R>) -> CmdResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("projects"))
        .map_err(|e| format!("no app data folder: {e}"))
}

/// A folder name for a project called `name`: its letters and digits, with
/// everything else as single dashes. Never empty and never a dot name.
pub(crate) fn project_slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

/// Create an empty folder for a project called `name` under `root` and write
/// its name into `project.json`. A name already taken gets `-2`, `-3`...
pub(crate) fn create_project_in(root: &Path, name: &str) -> CmdResult<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the project a name.".to_string());
    }
    if name.chars().count() > 80 {
        return Err("That name is too long (80 characters at most).".to_string());
    }
    std::fs::create_dir_all(root)
        .map_err(|e| format!("could not create the projects folder: {e}"))?;
    let slug = project_slug(name);
    let mut n = 1usize;
    let dir = loop {
        let candidate = if n == 1 {
            root.join(&slug)
        } else {
            root.join(format!("{slug}-{n}"))
        };
        // `create_dir`, not `create_dir_all`: it fails on a folder that is
        // already there, so two creates of one name cannot share a folder.
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(format!("could not create the project: {e}")),
        }
    };
    let mut meta = crate::project::read_meta(&dir);
    meta.name = name.to_string();
    crate::project::write_meta(&dir, &meta)
        .map_err(|e| format!("could not save the project's name: {e}"))?;
    Ok(dir)
}

/// Every project folder under `root`, most recently opened first.
pub(crate) fn list_projects_in(root: &Path) -> Vec<RecentProject> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut projects: Vec<RecentProject> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let path = entry.path();
            let meta = crate::project::read_meta(&path);
            RecentProject {
                path: path.to_string_lossy().into_owned(),
                name: meta.name,
                last_opened_at: meta.last_opened_at.or(meta.created_at),
            }
        })
        .collect();
    // Newest first; a project never opened sorts last, then by name so the
    // order does not depend on the directory listing.
    projects.sort_by(|a, b| {
        b.last_opened_at
            .cmp(&a.last_opened_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    projects
}

/// The projects kept in the app's storage, most recently opened first.
#[tauri::command]
pub async fn list_managed_projects<R: Runtime>(app: AppHandle<R>) -> CmdResult<Vec<RecentProject>> {
    let root = projects_root(&app)?;
    off_thread(move || Ok(list_projects_in(&root))).await
}

/// Make a folder for a new project in the app's storage and answer its
/// path, which `open_project` (a new project) or `save_project_as` (a copy
/// of the open one) then takes.
#[tauri::command]
pub async fn create_managed_project<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> CmdResult<String> {
    let root = projects_root(&app)?;
    off_thread(move || {
        create_project_in(&root, &name).map(|dir| dir.to_string_lossy().into_owned())
    })
    .await
}

/// Delete `project`, which must be a folder directly inside `root` and not
/// the project that is open.
pub(crate) fn delete_project_in(root: &Path, project: &Path, open: Option<&Path>) -> CmdResult<()> {
    let root = root
        .canonicalize()
        .map_err(|_| "There are no saved projects.".to_string())?;
    let project = project
        .canonicalize()
        .map_err(|_| "That project is no longer there.".to_string())?;
    if project.parent() != Some(root.as_path()) || !project.is_dir() {
        return Err("Only a project kept in the app can be deleted here.".to_string());
    }
    if let Some(open) = open.and_then(|open| open.canonicalize().ok()) {
        if open == project {
            return Err("That project is open. Open another one first.".to_string());
        }
    }
    // The window having left a project does not mean nothing is in it: a
    // turn that began there keeps its store until it ends. A private open
    // is refused while any store of the folder is alive, and while this one
    // lives nothing else can open the folder, so the delete has it alone.
    let claim =
        match session::Store::open_private(&project) {
            Ok(store) => store,
            Err(session::Error::AlreadyOpen { .. }) => return Err(
                "The assistant is still working in that project. Try again when it has finished."
                    .to_string(),
            ),
            Err(e) => return Err(format!("could not delete the project: {e}")),
        };
    let removed =
        std::fs::remove_dir_all(&project).map_err(|e| format!("could not delete the project: {e}"));
    drop(claim);
    removed
}

/// Delete a project kept in the app's storage, with all its audio and
/// history. Not the open one. Answers the projects that remain.
#[tauri::command]
pub async fn delete_managed_project<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<Vec<RecentProject>> {
    let root = projects_root(&app)?;
    let open = state
        .project_dir
        .lock()
        .map_err(|_| "the project lock is poisoned".to_string())?
        .clone();
    off_thread(move || {
        delete_project_in(&root, Path::new(&path), open.as_deref())?;
        // Its row in the recents list would be dead otherwise.
        if let Some(home) = crate::paths::home_base(&app) {
            let _ = crate::project::forget_recent(&home, &path);
        }
        Ok(list_projects_in(&root))
    })
    .await
}

// ---------------------------------------------------------------------------
// The back button and the microphone
// ---------------------------------------------------------------------------

/// Put the app behind the launcher, as the back button does at the top of
/// any Android app. The process goes on: a turn or a download that is
/// running is not ended. The window calls this when a press of Back has
/// nothing left to close.
#[tauri::command]
pub async fn leave_app<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    off_thread(move || {
        ask_android::<R, serde_json::Value>(&app, "leaveApp", serde_json::json!({}))?;
        Ok(())
    })
    .await
}

/// Whether the answer of `EdytlabPlugin`'s permission commands says the
/// microphone may be used. They answer `{ "microphone": "granted" }`, or
/// `"denied"`, or one of two "not asked yet" states.
pub(crate) fn microphone_granted(answer: &serde_json::Value) -> bool {
    answer.get("microphone").and_then(serde_json::Value::as_str) == Some("granted")
}

/// Make sure the app may use the microphone, asking the person if it has
/// not been asked. Answers whether it may. The window calls this before
/// each recording; once granted, it only reads the state and shows nothing.
///
/// On a desktop the operating system asks by itself when the device is
/// opened, so there is nothing to do and the answer is yes.
#[tauri::command]
pub async fn ensure_microphone<R: Runtime>(app: AppHandle<R>) -> CmdResult<bool> {
    if !cfg!(target_os = "android") {
        return Ok(true);
    }
    off_thread(move || {
        let wanted = serde_json::json!({ "permissions": ["microphone"] });
        let now: serde_json::Value = ask_android(&app, "checkPermissions", serde_json::json!({}))?;
        if microphone_granted(&now) {
            return Ok(true);
        }
        // Shows Android's own dialog and answers when it is dismissed. A
        // person who has refused twice gets no dialog and a plain "denied".
        let asked: serde_json::Value = ask_android(&app, "requestPermissions", wanted)?;
        Ok(microphone_granted(&asked))
    })
    .await
}

/// Where a recording called `name` is saved. The window names a take
/// (`recording_<time>.wav`) and leaves the folder to the backend. On a
/// desktop that is as it always was: the path as given. On Android a bare
/// name would land in the working folder, which is the exports folder, so
/// it goes into the open project's `imports/` beside the audio the person
/// brought in, where the project keeps it.
pub(crate) fn recording_destination(name: &str, project: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(name);
    match project {
        Some(project) if cfg!(target_os = "android") && path.is_relative() => {
            recording_in_project(project, &path)
        }
        _ => path,
    }
}

/// `<project>/imports/<name>`, with the folder made.
pub(crate) fn recording_in_project(project: &Path, name: &Path) -> PathBuf {
    let dir = project.join(IMPORTS_DIR);
    // A folder that cannot be made shows as the save failing, with its reason.
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name.file_name().unwrap_or(name.as_os_str()))
}

// ---------------------------------------------------------------------------
// Audio in: the picker's files
// ---------------------------------------------------------------------------

/// What `EdytlabPlugin.copyUriToFile` answers.
#[derive(Debug, Deserialize)]
struct CopiedFile {
    path: String,
}

/// Load the files the system picker named.
///
/// Each `content://` address is copied into the open project's `imports/`
/// folder under the name the picker showed, and the copies are loaded as
/// `batch_load` loads any file, one track each. A file that cannot be
/// copied or is not audio is reported in `failures` and does not stop the
/// others. A copy the loader refused is removed again.
#[tauri::command]
pub async fn import_audio<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    uris: Vec<String>,
) -> CmdResult<BatchLoadResult> {
    if uris.is_empty() {
        return Err("no files provided".to_string());
    }
    // On the blocking pool, as `batch_load` is: copying is slow, and the
    // load waits for the store, the engine and the dispatcher, which a
    // running tool can hold.
    crate::commands::blocking(&state, move |state| {
        let project = crate::commands::current_project_dir(state).map_err(|e| e.to_string())?;
        let imports = project.join(IMPORTS_DIR);

        let mut copied = Vec::new();
        let mut failures = Vec::new();
        for uri in &uris {
            match ask_android::<R, CopiedFile>(
                &app,
                "copyUriToFile",
                serde_json::json!({ "uri": uri, "dir": imports }),
            ) {
                Ok(file) => copied.push(file.path),
                Err(error) => failures.push(BatchLoadFailure {
                    path: uri.clone(),
                    error,
                }),
            }
        }
        if copied.is_empty() {
            return Ok(BatchLoadResult {
                tracks_loaded: 0,
                last_node_id: None,
                failures,
            });
        }

        let mut result = crate::commands::batch_load_inner(state, &copied)?;
        for refused in &result.failures {
            // Nothing in the project names it, so it would only take space.
            let _ = std::fs::remove_file(&refused.path);
        }
        failures.append(&mut result.failures);
        result.failures = failures;
        Ok(result)
    })
    .await
}

// ---------------------------------------------------------------------------
// Audio out: exports
// ---------------------------------------------------------------------------

/// `exports/` in the app's data folder.
pub(crate) fn exports_dir<R: Runtime, M: Manager<R>>(app: &M) -> CmdResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("exports"))
        .map_err(|e| format!("no app data folder: {e}"))
}

/// `name` as a file name that stays inside one folder: the last path
/// component, without control characters or the characters a share target
/// may refuse. `None` when nothing usable is left.
pub(crate) fn plain_file_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = last
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.chars().take(120).collect())
    }
}

/// A path in `dir` for an export called `name`: that name if it is free,
/// else `name (2)`, `name (3)`... before the extension.
pub(crate) fn free_export_path(dir: &Path, name: &str) -> CmdResult<PathBuf> {
    let name = plain_file_name(name).ok_or_else(|| "The export needs a file name.".to_string())?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("could not create the exports folder: {e}"))?;
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name.as_str(), ""),
    };
    let mut candidate = dir.join(&name);
    let mut n = 2usize;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    Ok(candidate)
}

/// The file in `dir` an export command was asked about, refused unless it is
/// a plain name of a file that is there.
pub(crate) fn export_file(dir: &Path, name: &str) -> CmdResult<PathBuf> {
    let plain = plain_file_name(name).filter(|plain| plain == name);
    let Some(plain) = plain else {
        return Err("That is not the name of an export.".to_string());
    };
    let path = dir.join(plain);
    if !path.is_file() {
        return Err(format!("{name} is no longer in the exports."));
    }
    Ok(path)
}

/// One file in the exports folder.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportFile {
    pub name: String,
    pub bytes: u64,
    /// Milliseconds since 1970, for the "when" the list shows.
    pub modified_ms: u64,
}

/// The files in `dir`, newest first. Writers' staging files (dot names) are
/// left out.
pub(crate) fn list_exports_in(dir: &Path) -> Vec<ExportFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<ExportFile> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let name = entry.file_name().to_str()?.to_string();
            if !meta.is_file() || name.starts_with('.') {
                return None;
            }
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some(ExportFile {
                name,
                bytes: meta.len(),
                modified_ms,
            })
        })
        .collect();
    files.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    files
}

/// Where an export called `name` should be written: a free path in the
/// app's exports folder. The Android window asks this instead of showing a
/// save dialog, then offers Share and Save to on the result.
#[tauri::command]
pub async fn export_destination<R: Runtime>(app: AppHandle<R>, name: String) -> CmdResult<String> {
    let dir = exports_dir(&app)?;
    off_thread(move || {
        free_export_path(&dir, &name).map(|path| path.to_string_lossy().into_owned())
    })
    .await
}

/// The files in the app's exports folder, newest first.
#[tauri::command]
pub async fn list_exports<R: Runtime>(app: AppHandle<R>) -> CmdResult<Vec<ExportFile>> {
    let dir = exports_dir(&app)?;
    off_thread(move || Ok(list_exports_in(&dir))).await
}

/// Remove one export. Answers the files that remain.
#[tauri::command]
pub async fn delete_export<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> CmdResult<Vec<ExportFile>> {
    let dir = exports_dir(&app)?;
    off_thread(move || {
        let path = export_file(&dir, &name)?;
        std::fs::remove_file(&path).map_err(|e| format!("could not delete {name}: {e}"))?;
        Ok(list_exports_in(&dir))
    })
    .await
}

/// Open the system share sheet for one export.
#[tauri::command]
pub async fn share_export<R: Runtime>(app: AppHandle<R>, name: String) -> CmdResult<()> {
    let dir = exports_dir(&app)?;
    off_thread(move || {
        let path = export_file(&dir, &name)?;
        ask_android::<R, serde_json::Value>(
            &app,
            "shareFile",
            serde_json::json!({ "path": path }),
        )?;
        Ok(())
    })
    .await
}

/// Write one export into the document the system's save dialog created at
/// `uri`.
#[tauri::command]
pub async fn save_export_to<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    uri: String,
) -> CmdResult<()> {
    let dir = exports_dir(&app)?;
    off_thread(move || {
        let path = export_file(&dir, &name)?;
        ask_android::<R, serde_json::Value>(
            &app,
            "copyFileToUri",
            serde_json::json!({ "path": path, "uri": uri }),
        )?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_note_names_the_exports_folder_and_is_absent_on_a_desktop() {
        let note = platform_note_for(Path::new("/data/user/0/app/exports"));
        assert!(note.contains("`/data/user/0/app/exports`"));
        assert!(note.contains("`imports`"));
        // Nothing sets the folder outside Android, so a profile's text is
        // passed through untouched.
        assert_eq!(
            with_platform_note(Some("body".into())).as_deref(),
            Some("body")
        );
        assert_eq!(with_platform_note(None), None);
    }

    #[test]
    fn only_an_explicit_null_says_that_no_secret_is_saved() {
        use serde_json::json;
        assert_eq!(secret_in(&json!({ "value": null })), Ok(None));
        assert_eq!(
            secret_in(&json!({ "value": "c2stMQ==" })),
            Ok(Some(b"sk-1".to_vec()))
        );
        // An answer that does not say is not "no key": the app would show
        // the first-run prompt over a key that is saved.
        for broken in [
            json!({}),
            json!(null),
            json!({ "value": 7 }),
            json!({ "value": "not base64 sk-secret" }),
        ] {
            let error = secret_in(&broken).expect_err("must not read as nothing saved");
            assert!(!error.contains("sk-secret"), "{error}");
        }
    }

    /// Off Android there is no Keystore to ask, and the store says so. It
    /// never answers "nothing saved" or "saved" for a call it could not make.
    #[test]
    fn the_keystore_store_is_an_error_everywhere_but_android() {
        use ai::keychain::SecretStore as _;
        let app = tauri::test::mock_app();
        let store = KeystoreSecrets(app.handle().clone());
        assert_eq!(store.get("secrets"), Err(UNAVAILABLE.to_string()));
        assert_eq!(store.set("secrets", b"{}"), Err(UNAVAILABLE.to_string()));
        assert_eq!(store.delete("secrets"), Err(UNAVAILABLE.to_string()));
    }

    #[test]
    fn the_screen_changes_only_when_the_first_work_starts_and_the_last_ends() {
        let busy = std::sync::atomic::AtomicU64::new(0);
        assert_ne!(busy_bit("turn"), busy_bit("download:htdemucs"));
        assert_eq!(note_busy(&busy, "turn", true), Some(true));
        assert_eq!(note_busy(&busy, "download:htdemucs", true), None);
        // Said twice, counted once.
        assert_eq!(note_busy(&busy, "turn", true), None);
        assert_eq!(note_busy(&busy, "turn", false), None);
        assert_eq!(note_busy(&busy, "download:htdemucs", false), Some(false));
        // Ending what never started changes nothing.
        assert_eq!(note_busy(&busy, "turn", false), None);
    }

    #[test]
    fn only_a_granted_microphone_is_a_yes() {
        use serde_json::json;
        assert!(microphone_granted(&json!({ "microphone": "granted" })));
        for no in [
            json!({ "microphone": "denied" }),
            json!({ "microphone": "prompt" }),
            json!({ "microphone": "prompt-with-rationale" }),
            json!({}),
            json!(null),
        ] {
            assert!(!microphone_granted(&no), "{no}");
        }
    }

    #[test]
    fn a_take_goes_into_the_projects_imports_folder_by_its_name_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p");
        let at = recording_in_project(&project, Path::new("recording_17.wav"));
        assert_eq!(at, project.join("imports").join("recording_17.wav"));
        assert!(project.join("imports").is_dir());
        // A name that tries to climb is cut to its last part.
        let climbing = recording_in_project(&project, Path::new("../../x/take.wav"));
        assert_eq!(climbing, project.join("imports").join("take.wav"));
        // Off Android the path is the caller's, as it always was.
        if !cfg!(target_os = "android") {
            assert_eq!(
                recording_destination("recording_17.wav", Some(&project)),
                PathBuf::from("recording_17.wav")
            );
        }
    }

    #[test]
    fn a_project_with_a_live_store_is_not_deleted_under_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        let busy = create_project_in(&root, "Busy").unwrap();
        // A turn that began in the project still holds its store.
        let held = session::Store::open_shared(&busy).unwrap();
        let refused = delete_project_in(&root, &busy, None).unwrap_err();
        assert!(refused.contains("still working"), "{refused}");
        assert!(busy.is_dir());
        drop(held);
        delete_project_in(&root, &busy, None).unwrap();
        assert!(!busy.exists());
    }

    #[test]
    fn a_slug_keeps_letters_and_digits_and_is_never_empty() {
        assert_eq!(
            project_slug("My Podcast, Episode 12!"),
            "my-podcast-episode-12"
        );
        assert_eq!(project_slug("  ../../etc  "), "etc");
        assert_eq!(project_slug("Été à Liège"), "été-à-liège");
        assert_eq!(project_slug("///"), "project");
        assert_eq!(project_slug(""), "project");
        assert!(project_slug(&"a".repeat(200)).chars().count() <= 60);
    }

    #[test]
    fn creating_a_project_names_it_and_never_reuses_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        let first = create_project_in(&root, "Interview").unwrap();
        let second = create_project_in(&root, "Interview").unwrap();
        assert_eq!(first, root.join("interview"));
        assert_eq!(second, root.join("interview-2"));
        assert_eq!(crate::project::read_meta(&first).name, "Interview");
        assert_eq!(crate::project::read_meta(&second).name, "Interview");
        assert!(create_project_in(&root, "   ").is_err());
        assert!(create_project_in(&root, &"x".repeat(81)).is_err());
    }

    #[test]
    fn projects_are_listed_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        assert!(list_projects_in(&root).is_empty());
        let old = create_project_in(&root, "Old").unwrap();
        let new = create_project_in(&root, "New").unwrap();
        let never = create_project_in(&root, "Never opened").unwrap();
        crate::project::touch_opened(&old, "2026-01-01T00:00:00Z");
        crate::project::touch_opened(&new, "2026-06-01T00:00:00Z");
        // A stray file in the folder is not a project.
        std::fs::write(root.join("note.txt"), b"x").unwrap();
        let names: Vec<_> = list_projects_in(&root)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["New", "Old", "Never opened"]);
        let _ = never;
    }

    #[test]
    fn only_a_closed_project_inside_the_root_is_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        let open = create_project_in(&root, "Open").unwrap();
        let closed = create_project_in(&root, "Closed").unwrap();
        let outside = tmp.path().join("elsewhere");
        std::fs::create_dir_all(outside.join("inner")).unwrap();

        assert!(delete_project_in(&root, &open, Some(&open)).is_err());
        assert!(delete_project_in(&root, &outside, Some(&open)).is_err());
        assert!(delete_project_in(&root, &root, Some(&open)).is_err());
        // A path that climbs out of the root and back is still judged by
        // where it ends up.
        let climbing = root.join("closed").join("..").join("..").join("elsewhere");
        assert!(delete_project_in(&root, &climbing, Some(&open)).is_err());
        assert!(outside.is_dir() && open.is_dir());

        delete_project_in(&root, &closed, Some(&open)).unwrap();
        assert!(!closed.exists());
        assert!(delete_project_in(&root, &closed, Some(&open)).is_err());
    }

    #[test]
    fn an_export_name_never_leaves_the_folder() {
        assert_eq!(plain_file_name("mix.wav").as_deref(), Some("mix.wav"));
        assert_eq!(
            plain_file_name("../../secret.wav").as_deref(),
            Some("secret.wav")
        );
        assert_eq!(
            plain_file_name("C:\\Users\\me\\mix.flac").as_deref(),
            Some("mix.flac")
        );
        assert_eq!(plain_file_name("a:b?.mp3").as_deref(), Some("a_b_.mp3"));
        assert_eq!(plain_file_name(".hidden").as_deref(), Some("hidden"));
        assert_eq!(plain_file_name("dir/"), None);
        assert_eq!(plain_file_name(""), None);
    }

    #[test]
    fn an_export_gets_a_free_name_and_is_found_only_by_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("exports");
        let first = free_export_path(&dir, "mix.wav").unwrap();
        assert_eq!(first, dir.join("mix.wav"));
        std::fs::write(&first, b"one").unwrap();
        let second = free_export_path(&dir, "/somewhere/else/mix.wav").unwrap();
        assert_eq!(second, dir.join("mix (2).wav"));
        std::fs::write(&second, b"three").unwrap();
        std::fs::write(dir.join(".mix.wav.part"), b"x").unwrap();

        let listed = list_exports_in(&dir);
        let mut names: Vec<_> = listed.iter().map(|f| f.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["mix (2).wav", "mix.wav"]);
        assert_eq!(
            listed.iter().find(|f| f.name == "mix.wav").unwrap().bytes,
            3
        );

        assert_eq!(export_file(&dir, "mix.wav").unwrap(), first);
        assert!(export_file(&dir, "../exports/mix.wav").is_err());
        assert!(export_file(&dir, "missing.wav").is_err());
        assert!(export_file(&dir, "").is_err());
    }
}
