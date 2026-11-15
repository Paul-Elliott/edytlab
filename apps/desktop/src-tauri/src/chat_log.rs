//! A project's conversation, kept with the project (the Android app).
//!
//! A desktop keeps the chat in memory, and that is enough there: the app
//! runs until it is closed. Android stops an app that is out of sight
//! whenever it wants the memory, so on a phone "until it is closed" is a few
//! minutes in another app. Without this the chat came back empty and the
//! assistant had forgotten the whole conversation, part way through a job.
//!
//! Two files in the project's `.audiograph/`, each with one writer:
//!
//! * [`VIEW_FILE`]: what the chat pane shows, as the window's own entries.
//!   Written by the window ([`save_chat_log`]), read back when the project
//!   opens ([`load_chat_log`]). This crate does not read inside an entry.
//! * [`AGENT_FILE`]: the messages the model is sent, without its thinking.
//!   Written at the end of each turn ([`keep_agent`]), and given to the
//!   agent built for the project ([`give_back`]), which keeps only what is
//!   safe to send again (`ai::Agent::restore_conversation`).
//!
//! Both are a convenience and are read that way: a missing, damaged or
//! oversized file is an empty conversation, never a project that does not
//! open. They are deleted with the project and are not copied by Duplicate.
//!
//! Android only ([`kept`]). On a desktop nothing here writes or reads a
//! file: a conversation on disk in a folder the person may share is a change
//! to what a desktop does, and needs its own decision.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::commands::{blocking, current_project_dir, CmdResult};
use crate::state::AppState;

/// What the chat pane shows.
pub const VIEW_FILE: &str = "chat.json";
/// What the model is sent.
pub const AGENT_FILE: &str = "chat-agent.json";

/// The most entries of the pane that are kept: the newest. A message, a
/// tool badge and a divider are each one.
pub const MAX_VIEW_ENTRIES: usize = 400;
/// The most either file may hold. A file over it is not read: it is not one
/// this module wrote.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Whether conversations are kept on this platform.
pub const fn kept() -> bool {
    cfg!(target_os = "android")
}

fn file(project_dir: &Path, name: &str) -> PathBuf {
    project_dir.join(session::STORE_DIR).join(name)
}

/// Read a JSON array from one of the two files. Empty for anything else.
fn read_list<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    let small_enough = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
    if !small_enough {
        return Vec::new();
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<T>>(&bytes).ok())
        .unwrap_or_default()
}

/// Write a JSON array beside its name and move it into place, so a write
/// that is cut off (the app stopped) leaves the last whole file.
fn write_list<T: Serialize>(path: &Path, list: &[T]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(list)?;
    let part = path.with_extension("json.part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}

/// The pane's entries as last saved for the project.
pub fn read_view(project_dir: &Path) -> Vec<serde_json::Value> {
    read_list(&file(project_dir, VIEW_FILE))
}

/// Save the pane's entries: the newest [`MAX_VIEW_ENTRIES`] of them.
pub fn write_view(project_dir: &Path, entries: &[serde_json::Value]) -> std::io::Result<()> {
    let from = entries.len().saturating_sub(MAX_VIEW_ENTRIES);
    write_list(&file(project_dir, VIEW_FILE), &entries[from..])
}

/// The model's messages as last saved for the project.
pub fn read_agent(project_dir: &Path) -> Vec<ai::anthropic::Message> {
    read_list(&file(project_dir, AGENT_FILE))
}

/// Save the model's messages.
pub fn write_agent(project_dir: &Path, messages: &[ai::anthropic::Message]) -> std::io::Result<()> {
    write_list(&file(project_dir, AGENT_FILE), messages)
}

/// Give an agent just built for `project_dir` the conversation saved there,
/// and note the folder as the one its turns are saved to. With `None` (no
/// project is open) the note is cleared.
///
/// Called with the agent's mutex held, by whoever builds the agent, so the
/// note and the agent in the slot always belong together.
pub fn give_back(state: &AppState, agent: &mut ai::Agent, project_dir: Option<PathBuf>) {
    give_back_where(kept(), state, agent, project_dir);
}

/// [`give_back`], with whether conversations are kept passed in, so a test
/// on a desktop can run what a phone runs.
pub(crate) fn give_back_where(
    kept: bool,
    state: &AppState,
    agent: &mut ai::Agent,
    project_dir: Option<PathBuf>,
) {
    if !kept {
        return;
    }
    if let Some(dir) = &project_dir {
        let taken = agent.restore_conversation(read_agent(dir));
        tracing::debug!(messages = taken, "took back the project's conversation");
    }
    state.note_agent_chat_dir(project_dir);
}

/// Save the agent's conversation to the project it was built for. Called at
/// the end of a turn, with the agent's mutex still held. A failure is
/// logged: the turn itself succeeded.
pub fn keep_agent(state: &AppState, agent: &ai::Agent) {
    keep_agent_where(kept(), state, agent);
}

/// [`keep_agent`], with whether conversations are kept passed in.
pub(crate) fn keep_agent_where(kept: bool, state: &AppState, agent: &ai::Agent) {
    if !kept {
        return;
    }
    let Some(dir) = state.agent_chat_dir() else {
        return;
    };
    if let Err(error) = write_agent(&dir, &agent.conversation_to_keep()) {
        tracing::warn!(%error, "could not save the conversation");
    }
}

/// Save the conversation as it stands with `text` about to be sent. Called
/// as a turn starts, with the agent's mutex held. A long tool (separating
/// stems) is when Android is most likely to stop the app, and a turn that
/// never ends is never saved by [`keep_agent`]: without this the assistant
/// came back with no idea what it had been asked.
pub fn keep_request(state: &AppState, agent: &ai::Agent, text: &str) {
    keep_request_where(kept(), state, agent, text);
}

/// [`keep_request`], with whether conversations are kept passed in.
pub(crate) fn keep_request_where(kept: bool, state: &AppState, agent: &ai::Agent, text: &str) {
    if !kept {
        return;
    }
    let Some(dir) = state.agent_chat_dir() else {
        return;
    };
    if let Err(error) = write_agent(&dir, &agent.conversation_to_keep_with(text)) {
        tracing::warn!(%error, "could not save the request");
    }
}

/// What [`load_chat_log`] answers.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatLog {
    /// The open project's folder: what [`save_chat_log`] is to be told the
    /// entries belong to.
    pub project: String,
    /// The pane's entries, oldest first. Empty where conversations are not
    /// kept, and for a project with none saved.
    pub entries: Vec<serde_json::Value>,
}

/// The chat pane's entries saved for the open project (the Android app).
///
/// On the blocking pool: it reads a file.
#[tauri::command]
pub async fn load_chat_log(state: State<'_, AppState>) -> CmdResult<ChatLog> {
    blocking(&state, |state| {
        let dir = current_project_dir(state)?;
        let entries = if kept() { read_view(&dir) } else { Vec::new() };
        Ok(ChatLog {
            project: dir.to_string_lossy().into_owned(),
            entries,
        })
    })
    .await
}

/// Save the chat pane's entries for `project` (the Android app).
///
/// `project` is the folder [`load_chat_log`] named. If another project has
/// been opened since, nothing is written: the entries are not that
/// project's. `false` when nothing was written, which is also the answer
/// where conversations are not kept.
#[tauri::command]
pub async fn save_chat_log(
    state: State<'_, AppState>,
    project: String,
    entries: Vec<serde_json::Value>,
) -> CmdResult<bool> {
    blocking(&state, move |state| {
        let dir = current_project_dir(state)?;
        if !kept() || dir != Path::new(&project) {
            return Ok(false);
        }
        write_view(&dir, &entries)
            .map_err(crate::commands::CommandError::from)
            .map_err(String::from)?;
        Ok(true)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai::anthropic::{ContentBlock, Message, Role};
    use serde_json::json;

    #[test]
    fn the_pane_reads_back_what_was_saved_newest_entries_only() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_view(tmp.path()).is_empty());

        let entries: Vec<_> = (0..MAX_VIEW_ENTRIES + 25)
            .map(|i| json!({ "kind": "message", "id": format!("m{i}"), "role": "user", "text": "hi" }))
            .collect();
        write_view(tmp.path(), &entries).unwrap();
        let read = read_view(tmp.path());
        assert_eq!(read.len(), MAX_VIEW_ENTRIES);
        assert_eq!(read[0]["id"], "m25");
        assert_eq!(
            read.last().unwrap()["id"],
            format!("m{}", MAX_VIEW_ENTRIES + 24)
        );

        // In the project's own store folder, with nothing left beside it.
        let dir = tmp.path().join(session::STORE_DIR);
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [VIEW_FILE]);
    }

    #[test]
    fn the_models_messages_read_back_as_they_were_saved() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_agent(tmp.path()).is_empty());
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "make it louder".into(),
                }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Done.".into(),
                }],
            },
        ];
        write_agent(tmp.path(), &messages).unwrap();
        let read = read_agent(tmp.path());
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(&messages).unwrap()
        );
    }

    #[test]
    fn a_file_that_is_not_ours_is_an_empty_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(session::STORE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        for bad in ["", "not json", "{}", "[1, 2", "{\"entries\": []}"] {
            std::fs::write(dir.join(VIEW_FILE), bad).unwrap();
            std::fs::write(dir.join(AGENT_FILE), bad).unwrap();
            assert!(read_view(tmp.path()).is_empty(), "{bad:?}");
            assert!(read_agent(tmp.path()).is_empty(), "{bad:?}");
        }
        // Entries of the wrong shape for the model are not half-read.
        std::fs::write(dir.join(AGENT_FILE), "[{\"role\": \"wizard\"}]").unwrap();
        assert!(read_agent(tmp.path()).is_empty());
    }

    #[test]
    fn a_file_over_the_limit_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(session::STORE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let big = format!("[\"{}\"]", "x".repeat(MAX_FILE_BYTES as usize));
        std::fs::write(dir.join(VIEW_FILE), big).unwrap();
        assert!(read_view(tmp.path()).is_empty());
    }

    /// Off Android nothing is kept: the two halves of the agent's side do
    /// nothing, so a desktop's project folder gains no file.
    #[test]
    fn conversations_are_kept_on_android_only() {
        assert_eq!(kept(), cfg!(target_os = "android"));
    }
}
