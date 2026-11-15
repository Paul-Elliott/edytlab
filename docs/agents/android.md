# Android

The Android app is the desktop app built for another target: the same Rust core and the same React frontend, through Tauri's Android support. This guide says what differs, where it lives, and how it is checked. Read it before changing anything it names.

## This fork

This repository (`Paul-Elliott/edytlab`) is a fork of `laadtushar/edytlab`. `CLAUDE.md` was written for upstream and its owner. What differs here:

- Pull requests are opened as drafts and merged by the fork's owner. Upstream's rule to squash-merge a green PR does not apply.
- Upstream's tracking issue (#366), release day steps, advisory bots, and site and marketing sweeps are upstream's process. Skip them unless asked.
- The fork's own check is the `Android` workflow (`.github/workflows/android.yml`). Upstream's desktop workflows are expected to be disabled. While they still run, three of their failures do not come from a change: the Linux build and the Rust coverage job run out of disk on a hosted runner, the coverage report has no DeepSource account to send to, and `Audit` reports advisories against upstream's lock file.
- The code conventions, the area guides and the acceptance gates apply as written.

## The rule

Nothing forks. Every Android difference is one of:

- Rust behind `cfg(target_os = "android")` (or `cfg(desktop)` for what only a desktop has, such as the native menu);
- the frontend's run-time checks in `apps/desktop/src/lib/platform.ts` (`isAndroid()`, `useFormFactor()`), read from the user agent, so one bundle serves every platform;
- files under `apps/desktop/src-tauri/gen/android/`, which only an Android build reads.

A change that alters what a desktop does is a desktop change and needs its own reason.

## Building

```sh
# once: Android SDK command-line tools, NDK 27, and the Rust targets
rustup target add aarch64-linux-android x86_64-linux-android
export ANDROID_HOME=... NDK_HOME=$ANDROID_HOME/ndk/<version>

pnpm --filter @edytlab/desktop tauri android build --apk --debug --target aarch64
```

- `gen/android/` is committed. It was made by `tauri android init` and then edited by hand: `MainActivity.kt`, `EdytlabPlugin.kt`, the manifest, `app/build.gradle.kts`, the themes. Do not regenerate it over those edits.
- `tauri.android.conf.json` gives the Android app its own id (`app.edytlab.android`) and `minSdkVersion` 26, which the recorder's AAudio backend needs.
- One key, `gen/android/app/sideload.keystore`, signs every build, debug or release, so a newer APK installs over an older one and keeps its projects. It is committed on purpose and protects nothing. It must never sign a Play Store upload.
- A debug APK allows WebView debugging and cleartext HTTP. The CI smoke test needs the first.
- `.github/workflows/android.yml` builds `arm64-v8a` and `x86_64` and runs the emulator test below.

## Where things are

| Desktop | Android | Code |
|---|---|---|
| `~/.edytlab` (memory, skills, agents, settings, MCP, recents) | `.edytlab` in the app's data folder. `MainActivity` also sets `HOME` and `TMPDIR` inside the app's storage before Rust starts | `paths.rs` (`home_base`), `MainActivity.kt` |
| A project is any folder the person picks | Projects are folders under `projects/` in the app's data, chosen from a list. "Save as" is "Duplicate". A project is deleted only under a private store, so one a turn still works in is refused | `android.rs`, `components/mobile/ProjectsSheet.tsx` |
| Open Audio gives paths | The system picker gives `content://` addresses. `import_audio` copies each into the project's `imports/` and loads the copy | `android.rs`, `EdytlabPlugin.copyUriToFile` |
| Export to a chosen path | Everything is written to `exports/` in the app's data, then shared or saved from the Exports sheet. The working folder is `exports/`, and the model is told to write there | `android.rs` (`platform_note_for`), `components/mobile/ExportsSheet.tsx` |
| Resources in `resource_dir()` | Package assets have no path, so the build embeds them and writes them to `resources/<version>/` at launch | `resources.rs`, `build.rs` |
| Keys in the OS keychain (`keyring`) | A file per account in the no-backup folder, AES-256-GCM under an Android Keystore key. `keyring` has no Android store and would forget every key | `crates/ai/src/keychain_platform_store.rs`, `EdytlabPlugin.secret*`, `docs/agents/providers.md` |
| ONNX Runtime from the installer | The `onnxruntime-android` package, same pinned version. Found beside the app's own library through `/proc/self/maps`; needs `useLegacyPackaging` so the library is a file | `ml-pipeline/src/runtime.rs` (`loaded_library_dir`), `lib.rs`, `ml-runtime.json` (`android`) |
| M4A by AudioToolbox or Media Foundation | NDK MediaExtractor and MediaCodec | `audio-decoder/src/os_aac/android.rs`, `docs/agents/formats.md` |
| Lanes decode each file in the webview | Lanes draw from `waveform_peaks` and the mix players stream. No snap-to-zero and no spectrogram, which need samples | `peaks.rs`, `Timeline.tsx` |
| Record | The same recorder (cpal on AAudio), after `ensure_microphone`. A take is saved into the project's `imports/` | `android.rs`, `EdytlabPlugin` permissions |
| The downloader and the stem cache lock a file with `std::fs::File::lock` | `flock` called directly. The pinned toolchain does not build the standard library's file locks for Android: they compile and answer "not supported" on a device | `ml-pipeline/src/file_lock.rs` |
| The app starts on an empty project, with the others under Recent | It reopens the project that was open last (`App.tsx`, the newest of the recents). Android stops an app that is out of sight, so most starts are a return part way through a job | `App.tsx` |
| The chat lives in memory until the app is closed | The chat is kept with the project, in two files in `.audiograph/`: `chat.json` (what the pane shows, written by the window) and `chat-agent.json` (what the model is sent, without its thinking, written at the end of each turn). An agent built for a project starts from the second, keeping only what is safe to send again, and no more than 200 KiB of it. A desktop reads and writes neither | `chat_log.rs`, `crates/ai/src/conversation.rs` (`restore`), `lib/chatLog.ts`, `Chat.tsx` |
| MCP servers over stdio | Refused with a reason: Android runs no programs for an app. HTTP servers work | `crates/mcp/src/transport.rs` |
| ChatGPT plan sign-in | Not offered. Its loopback redirect is lost when Android freezes the app behind the browser | `Settings.tsx` |
| Native menu, drag and drop, Ctrl shortcuts | A compact header, a menu sheet, and on-screen controls for undo, redo, delete and cut | `components/mobile/` |

## The window

- A phone shows one pane with a tab bar (Chat, Timeline, Transcript, History). A tablet at least 840 px wide and 520 px tall shows the desktop's two panes. Both panes stay mounted on a phone, stacked, with the hidden one `inert`, so the chat keeps its conversation and the timeline its width.
- `MainActivity` pads the WebView's container by the system bars, the display cutout and the keyboard. The page does not use CSS safe-area insets, whose behaviour depends on the WebView version.
- Touch: a finger dragged along a lane selects and a tap seeks; two fingers pinch to zoom and pan. Clips and envelope points drag (`touch-action: none`). A press and hold is the browser's `contextmenu`, which the existing right-click handlers already answer. In the transcript a tap selects a word and a second tap extends the selection, with a Cut bar, because a drag scrolls.
- Back closes the top dialog, then returns a phone to Chat, then puts the app behind the launcher without ending it (`useBackButton`, `leave_app`).
- The screen is kept on for the length of a turn and of a model download (`android::set_busy`). Android stops an app's work soon after the screen goes off.

## Kotlin

`EdytlabPlugin.kt` is the only Kotlin the app adds. Rust calls it through `android::ask_android`. It defines no command the webview can invoke, so the page gains no way to read or write a file: it hands a picker's address to a Rust command, and that command decides where the bytes go. Keep it that way. A new need from Android is a new `@Command` there and a Rust function that calls it.

## Stem separation

The model peaks near 3 GB. A desktop that is short gets an allocation failure the tool reports. Android's low-memory killer closes the app with no message. So on Android `separate_stems` refuses before loading the model when the device has under 7 GiB in total (`ANDROID_MIN_TOTAL_RAM_BYTES`: what a phone sold as 8 GB reports), and Settings does not offer the model there (`device_info.stems_supported`).

## Checking it

No test in this repo runs on a physical device. What stands in:

- **Host tests** cover the path logic (`android.rs`, `resources.rs`, `peaks.rs`), the keychain adapter with a fake store, the M4A trim arithmetic, the memory floor and the runtime's library lookup.
- **Cross-compiling** is the type check of the Android-only Rust: `cargo check --target aarch64-linux-android` for the crates, and a `tauri android build` for the app crate, which needs the CLI's environment.
- **Playwright `phone` and `tablet` projects** (`e2e/mobile-*.spec.ts`) run the production bundle with an Android user agent, a device viewport and real touch events against the fake backend: layout, sheets, import, exports, back, touch selection, pinch, transcript cut, recording permission, peaks, the project reopened at launch, the saved chat, and the first screen upright and sideways.
- **The emulator smoke test** (`scripts/ci/android-smoke.sh` and `.mjs`, run by `android.yml`) installs the APK and calls the real commands over the WebView's debugging socket: resources, the ONNX Runtime library, a model download from Hugging Face that is still there after a force-stop, a key that survives one too, the project reopened and its chat on screen after one, WAV and M4A import, playback from the asset protocol, export, a recording, Back.

Still needing a device: stem separation and transcription speed and memory, recording quality and latency, the system picker with real file managers, and how the layout sits around a notch.
