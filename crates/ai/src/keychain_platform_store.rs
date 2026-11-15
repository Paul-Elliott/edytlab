//! A secret store the app supplies, for a platform `keyring` has no store for.
//!
//! `keyring` 3 has no Android backend. Built for Android it compiles, and
//! every `Entry` is an item of its in-memory mock store: a saved key is gone
//! at the next launch, while nothing says so. On Android the app therefore
//! gives this module a store of its own with [`install_platform_store`]: the
//! Android Keystore, reached through the app's Kotlin plugin
//! (`apps/desktop/src-tauri/src/android.rs`, `EdytlabPlugin.kt`).
//! [`PlatformSlot`] presents that store to the logic in `keychain.rs` as a
//! [`Slot`], so the one `secrets` item, its cache, the lock and the migration
//! work there exactly as they do over an OS keychain. There is no second
//! store beside it (no `legacy` slot).
//!
//! # Errors
//!
//! - Nothing saved under an account is `keyring::Error::NoEntry`, which the
//!   logic reads as "no key".
//! - A store that is not installed yet is `NoStorageAccess`: an access
//!   failure, as a locked keychain is. It is never "no key" (the app would
//!   show the first-run prompt over a key that is saved), it is not
//!   remembered, a migration it stops is asked for again at the next call
//!   rather than leaving the process on per-slot items, and a save fails
//!   rather than landing nowhere.
//! - A store that answers with an error (a file that will not decrypt, a key
//!   the Keystore no longer has, a failed write) is `PlatformFailure` with the
//!   store's own words, which must never hold a value.
//!
//! Everything here compiles on every platform, so the host tests cover it
//! against a store in memory. Only the choice of it, in `os_slots` and in
//! `Store::persistence`, is Android's.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use super::{Persistence, Slot};

/// A place the app keeps secrets in, on a platform whose own keychain this
/// crate cannot reach. One value per account name; the names are the
/// keychain accounts of `keychain.rs` (`secrets` for the one item).
///
/// An error is the reason as text for a log and for the person. It must
/// never contain a stored value.
pub trait SecretStore: Send + Sync {
    /// The bytes saved under `account`, or `None` when nothing is saved
    /// there. A value that is there and cannot be read is an `Err`, never
    /// `None`: `None` tells the app the person has no key.
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, String>;

    /// Save `value` under `account`, replacing what was there. `Ok` means it
    /// will still be there after the app restarts.
    fn set(&self, account: &str, value: &[u8]) -> Result<(), String>;

    /// Remove what is saved under `account`. Nothing saved there is success.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The store [`install_platform_store`] was given.
static PLATFORM_STORE: OnceLock<Box<dyn SecretStore>> = OnceLock::new();

/// Whether the installed store has kept a value once ([`probe`]).
static ROUND_TRIP_WORKED: AtomicBool = AtomicBool::new(false);

/// Give this module the app's own secret store, once, at startup, before
/// anything reads a key. On Android every secret is then kept in it; until it
/// is installed every read and save there fails as an access failure (see the
/// module docs). On every other platform the OS keychain is used and an
/// installed store is never asked.
///
/// Returns whether this call installed it. It does not when a store was
/// installed already: the first one stands.
pub fn install_platform_store(store: Box<dyn SecretStore>) -> bool {
    PLATFORM_STORE.set(store).is_ok()
}

#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
fn installed() -> Option<&'static dyn SecretStore> {
    PLATFORM_STORE.get().map(|store| &**store)
}

/// A secret's bytes as text, for a store reached over a bridge that carries
/// text (the Kotlin plugin takes JSON): standard base64, padded.
pub fn secret_to_text(value: &[u8]) -> String {
    STANDARD.encode(value)
}

/// The bytes [`secret_to_text`] wrote. The error says nothing of the text: a
/// decoder's own words name the byte it stopped at.
pub fn secret_from_text(text: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(text)
        .map_err(|_| "a saved secret did not arrive whole".to_string())
}

/// What a call answers while no store is installed.
const NOT_INSTALLED: &str = "the app's secret store is not set up yet";

/// One account of a [`SecretStore`], as the store logic sees a keychain
/// entry. `store` is `None` while none is installed.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(super) struct PlatformSlot<'a> {
    store: Option<&'a dyn SecretStore>,
    account: String,
}

#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
impl PlatformSlot<'static> {
    /// `account` of the installed store. Looked up at each open, so a call
    /// made before the store is installed fails and the next one does not.
    pub(super) fn open(account: &str) -> Self {
        Self::of(installed(), account)
    }
}

#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
impl<'a> PlatformSlot<'a> {
    fn of(store: Option<&'a dyn SecretStore>, account: &str) -> Self {
        Self {
            store,
            account: account.to_string(),
        }
    }

    fn store(&self) -> keyring::Result<&'a dyn SecretStore> {
        self.store
            .ok_or_else(|| keyring::Error::NoStorageAccess(NOT_INSTALLED.into()))
    }
}

/// The store answered with an error.
fn failed(reason: String) -> keyring::Error {
    keyring::Error::PlatformFailure(reason.into())
}

impl Slot for PlatformSlot<'_> {
    fn get(&self) -> keyring::Result<String> {
        // Not `BadEncoding`: it carries the bytes, and they are the secret.
        String::from_utf8(self.get_secret()?)
            .map_err(|_| keyring::Error::PlatformFailure("the stored entry is not text".into()))
    }

    fn set(&self, value: &str) -> keyring::Result<()> {
        self.set_secret(value.as_bytes())
    }

    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        match self.store()?.get(&self.account) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(keyring::Error::NoEntry),
            Err(reason) => Err(failed(reason)),
        }
    }

    fn set_secret(&self, value: &[u8]) -> keyring::Result<()> {
        self.store()?.set(&self.account, value).map_err(failed)
    }

    fn delete(&self) -> keyring::Result<()> {
        self.store()?.delete(&self.account).map_err(failed)
    }
}

/// The account [`probe`] saves to, reads and removes. No slot has this name
/// (a test pins it), and it is not `PROBE_ACCOUNT`, which nothing may write.
const ROUND_TRIP_ACCOUNT: &str = "persistence_round_trip";

/// Whether a secret saved now will be there after a restart, for the
/// installed store: what `persistence()` answers on Android.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(super) fn persistence() -> Persistence {
    probe(installed(), &ROUND_TRIP_WORKED)
}

/// Durable when `store` is installed and keeps a value: one is saved, read
/// back and removed. Anything else is [`Persistence::SessionOnly`] with the
/// reason, the one answer the type has for "not durable"; on such a store a
/// save fails rather than landing somewhere that is lost.
///
/// A store that kept a value once is not asked again (`worked`): unlike a
/// Secret Service it does not come and go while the app runs, and a save
/// that fails later says so itself.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
fn probe(store: Option<&dyn SecretStore>, worked: &AtomicBool) -> Persistence {
    let Some(store) = store else {
        return Persistence::SessionOnly {
            reason: NOT_INSTALLED.to_string(),
        };
    };
    if worked.load(Ordering::Relaxed) {
        return Persistence::Durable;
    }
    match round_trip(store) {
        Ok(()) => {
            worked.store(true, Ordering::Relaxed);
            Persistence::Durable
        }
        Err(reason) => Persistence::SessionOnly { reason },
    }
}

/// Save a value that is new each time, read it back, and remove it.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
fn round_trip(store: &dyn SecretStore) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let value = format!("round-trip-{}-{nanos}", std::process::id()).into_bytes();
    store.set(ROUND_TRIP_ACCOUNT, &value)?;
    let back = store.get(ROUND_TRIP_ACCOUNT);
    // Whatever was read: the value is not a secret, and one left behind
    // would only take space.
    let _ = store.delete(ROUND_TRIP_ACCOUNT);
    match back? {
        Some(read) if read == value => Ok(()),
        Some(_) => Err("a value saved to the secret store read back changed".to_string()),
        None => Err("a value saved to the secret store was not there to read".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    use super::super::{
        account_for, base_url_account_for, effort_account_for, model_account_for, read_from,
        DirSource, Field, KeychainError, Layout, Store, ACTIVE_PROVIDER_ACCOUNT,
        CHATGPT_REFRESH_TOKEN_ACCOUNT, MIGRATED_MARKER, PROBE_ACCOUNT, SECRETS_ACCOUNT,
    };
    use super::*;

    /// What a [`Memory`] store does wrong, if anything.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        /// Every call answers this error: a Keystore key that is gone.
        Fails(&'static str),
        /// A save answers `Ok` and keeps nothing.
        Forgets,
        /// A read answers something other than what was saved.
        Garbles,
    }

    /// A [`SecretStore`] in memory that counts its calls.
    struct Memory {
        items: Mutex<BTreeMap<String, Vec<u8>>>,
        fault: Mutex<Fault>,
        calls: AtomicUsize,
    }

    impl Memory {
        fn new() -> Self {
            Self {
                items: Mutex::new(BTreeMap::new()),
                fault: Mutex::new(Fault::None),
                calls: AtomicUsize::new(0),
            }
        }

        fn with(fault: Fault) -> Self {
            let store = Self::new();
            *store.fault.lock().unwrap() = fault;
            store
        }

        /// One that outlives the test, for a `Store`'s slot factory.
        fn leaked() -> &'static Self {
            Box::leak(Box::new(Self::new()))
        }

        fn item(&self, account: &str) -> Option<Vec<u8>> {
            self.items.lock().unwrap().get(account).cloned()
        }

        fn accounts(&self) -> Vec<String> {
            self.items.lock().unwrap().keys().cloned().collect()
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }

        fn begin(&self) -> Result<Fault, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match *self.fault.lock().unwrap() {
                Fault::Fails(why) => Err(why.to_string()),
                fault => Ok(fault),
            }
        }
    }

    impl SecretStore for Memory {
        fn get(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
            let fault = self.begin()?;
            let found = self.item(account);
            Ok(match fault {
                Fault::Garbles => found.map(|_| b"something else".to_vec()),
                _ => found,
            })
        }

        fn set(&self, account: &str, value: &[u8]) -> Result<(), String> {
            if self.begin()? != Fault::Forgets {
                self.items
                    .lock()
                    .unwrap()
                    .insert(account.to_string(), value.to_vec());
            }
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), String> {
            self.begin()?;
            self.items.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn slot<'a>(store: &'a Memory, account: &str) -> PlatformSlot<'a> {
        PlatformSlot::of(Some(store), account)
    }

    fn read(slot: &dyn Slot) -> keyring::Result<Option<String>> {
        read_from(slot, None, &AtomicBool::new(false), &AtomicBool::new(false))
    }

    // --- the slot ---

    #[test]
    fn a_value_saved_is_read_back_as_text_and_as_bytes() {
        let store = Memory::new();
        let entry = slot(&store, "anthropic_api_key");
        entry.set("sk-ant-1").unwrap();
        assert_eq!(entry.get().unwrap(), "sk-ant-1");
        assert_eq!(entry.get_secret().unwrap(), b"sk-ant-1");
        assert_eq!(read(&entry).unwrap().as_deref(), Some("sk-ant-1"));

        // The bytes are kept as they are, under the account's own name.
        entry.set_secret("clé-ü-✓".as_bytes()).unwrap();
        assert_eq!(
            store.item("anthropic_api_key").as_deref(),
            Some("clé-ü-✓".as_bytes())
        );
        assert_eq!(entry.get().unwrap(), "clé-ü-✓");

        // And one account is not another's.
        assert!(matches!(
            slot(&store, "openai_api_key").get(),
            Err(keyring::Error::NoEntry)
        ));
    }

    #[test]
    fn nothing_saved_is_no_entry_which_reads_as_no_key() {
        let store = Memory::new();
        let entry = slot(&store, SECRETS_ACCOUNT);
        assert!(matches!(entry.get(), Err(keyring::Error::NoEntry)));
        assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
        assert_eq!(read(&entry).unwrap(), None);
    }

    #[test]
    fn a_delete_removes_the_value_and_a_missing_one_is_not_an_error() {
        let store = Memory::new();
        let entry = slot(&store, SECRETS_ACCOUNT);
        entry.set("{}").unwrap();
        entry.delete().unwrap();
        assert_eq!(store.item(SECRETS_ACCOUNT), None);
        assert!(matches!(entry.get(), Err(keyring::Error::NoEntry)));
        entry
            .delete()
            .expect("nothing there is the state asked for");
    }

    /// A store that answers with an error (a file that will not decrypt, a
    /// Keystore key that is gone) is a failure with the store's words. It
    /// must not read as "no key", or the first-run prompt would be shown
    /// over a key that is saved.
    #[test]
    fn a_store_error_is_a_platform_failure_and_never_a_missing_value() {
        let store = Memory::with(Fault::Fails("the saved secret could not be decrypted"));
        let entry = slot(&store, SECRETS_ACCOUNT);
        for failure in [
            entry.get().map(|_| ()).unwrap_err(),
            entry.get_secret().map(|_| ()).unwrap_err(),
            entry.set("x").unwrap_err(),
            entry.set_secret(b"x").unwrap_err(),
            entry.delete().unwrap_err(),
            read(&entry).map(|_| ()).unwrap_err(),
        ] {
            assert!(
                matches!(failure, keyring::Error::PlatformFailure(_)),
                "{failure}"
            );
            let shown = KeychainError::from(failure);
            assert!(shown.reason().contains("could not be decrypted"), "{shown}");
            assert!(!shown.is_access_denied());
        }
    }

    /// The `secrets` item is JSON, so it is text. Bytes that are not are an
    /// error that carries none of them (`BadEncoding` would).
    #[test]
    fn a_value_that_is_not_text_is_an_error_without_its_bytes() {
        let store = Memory::new();
        let entry = slot(&store, SECRETS_ACCOUNT);
        entry.set_secret(&[0xff, 0xfe, b's', b'k']).unwrap();
        assert_eq!(entry.get_secret().unwrap(), [0xff, 0xfe, b's', b'k']);
        let failure = entry.get().unwrap_err();
        assert!(matches!(failure, keyring::Error::PlatformFailure(_)));
        assert!(!format!("{failure:?}").contains("255"), "{failure:?}");
    }

    /// Before the app has installed its store there is nothing to ask. That
    /// is an access failure, as a locked keychain is: an error from every
    /// call, the kind a caller stops at, and never "no key".
    #[test]
    fn a_store_that_is_not_installed_refuses_every_call() {
        let entry = PlatformSlot::of(None, SECRETS_ACCOUNT);
        for failure in [
            entry.get().map(|_| ()).unwrap_err(),
            entry.get_secret().map(|_| ()).unwrap_err(),
            entry.set("x").unwrap_err(),
            entry.set_secret(b"x").unwrap_err(),
            entry.delete().unwrap_err(),
            read(&entry).map(|_| ()).unwrap_err(),
        ] {
            assert!(
                matches!(failure, keyring::Error::NoStorageAccess(_)),
                "{failure}"
            );
            let shown = KeychainError::from(failure);
            assert!(shown.is_access_denied());
            assert!(shown.reason().contains("not set up yet"), "{shown}");
        }
    }

    /// The one test that touches the process's own store. Nothing else in
    /// the host tests reads it: `os_slots` uses it on Android only.
    #[test]
    fn the_first_store_installed_stands_and_is_the_one_opened() {
        assert!(matches!(
            PlatformSlot::open("installed_probe").get(),
            Err(keyring::Error::NoStorageAccess(_))
        ));
        assert!(install_platform_store(Box::new(Memory::new())));
        let entry = PlatformSlot::open("installed_probe");
        entry.set("kept").unwrap();
        assert!(
            !install_platform_store(Box::new(Memory::new())),
            "a second store is refused"
        );
        assert_eq!(
            PlatformSlot::open("installed_probe").get().unwrap(),
            "kept",
            "and the first is still the one asked"
        );
        entry.delete().unwrap();
    }

    // --- the store logic over it ---

    /// A `Store` whose keychain is whatever `source` holds, with no second
    /// store beside it: what `os_slots` builds on Android.
    fn store_over(dir: &Path, source: &Arc<Mutex<Option<&'static Memory>>>) -> Store {
        let source = Arc::clone(source);
        Store::new(
            DirSource::Fixed(dir.to_path_buf()),
            Box::new(move |account| {
                let store = (*source.lock().unwrap()).map(|store| store as &dyn SecretStore);
                let primary: Box<dyn Slot> = Box::new(PlatformSlot::of(store, account));
                Ok((primary, None))
            }),
        )
    }

    fn secrets_of(memory: &Memory) -> BTreeMap<String, String> {
        let bytes = memory.item(SECRETS_ACCOUNT).expect("a secrets item");
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The bug this module is for: a key saved in one launch has to be
    /// there in the next. Each `Store` is a launch; the memory is what the
    /// device keeps.
    #[test]
    fn a_key_saved_in_one_launch_is_read_in_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::leaked();
        let source = Arc::new(Mutex::new(Some(memory)));

        let first = store_over(dir.path(), &source);
        assert_eq!(first.get(Field::ApiKey("anthropic")).unwrap(), None);
        assert_eq!(
            first.put(Field::ApiKey("anthropic"), "sk-ant-1").unwrap(),
            Persistence::Durable
        );
        first.put(Field::ChatGptToken, "refresh-1").unwrap();
        // One item holds both, as on every other platform.
        assert_eq!(memory.accounts(), [SECRETS_ACCOUNT]);
        let mut saved = secrets_of(memory);
        saved.remove(MIGRATED_MARKER);
        assert_eq!(
            saved,
            BTreeMap::from([
                ("anthropic_api_key".to_string(), "sk-ant-1".to_string()),
                ("chatgpt_refresh_token".to_string(), "refresh-1".to_string()),
            ])
        );

        let second = store_over(dir.path(), &source);
        assert_eq!(
            second.get(Field::ApiKey("anthropic")).unwrap().as_deref(),
            Some("sk-ant-1")
        );
        assert_eq!(
            second.get(Field::ChatGptToken).unwrap().as_deref(),
            Some("refresh-1")
        );

        // Removing the last secret removes the item.
        second.remove(Field::ApiKey("anthropic")).unwrap();
        second.remove(Field::ChatGptToken).unwrap();
        assert!(memory.accounts().is_empty());
        let third = store_over(dir.path(), &source);
        assert_eq!(third.get(Field::ApiKey("anthropic")).unwrap(), None);
    }

    /// A read or a save made before the store is installed is refused, and
    /// leaves the process able to use the store once it is there: it is not
    /// taken for an empty keychain, and the layout is not settled on the
    /// per-slot items.
    #[test]
    fn a_call_before_the_store_is_installed_is_refused_and_the_next_one_works() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::leaked();
        // What an earlier launch saved.
        let earlier = store_over(dir.path(), &Arc::new(Mutex::new(Some(memory))));
        earlier.put(Field::ApiKey("openai"), "sk-open-1").unwrap();

        for settings_file_is_there in [true, false] {
            if !settings_file_is_there {
                std::fs::remove_file(dir.path().join(super::super::SETTINGS_FILE)).unwrap();
            }
            let source = Arc::new(Mutex::new(None));
            let launch = store_over(dir.path(), &source);
            let refused = launch
                .get(Field::ApiKey("openai"))
                .expect_err("not installed must not read as no key");
            assert!(refused.is_access_denied(), "{refused}");
            let refused = launch
                .put(Field::ApiKey("openai"), "sk-open-2")
                .expect_err("and a save must not land nowhere");
            assert!(matches!(refused, keyring::Error::NoStorageAccess(_)));
            assert_ne!(launch.lock().layout, Layout::Legacy);

            *source.lock().unwrap() = Some(memory);
            assert_eq!(
                launch.get(Field::ApiKey("openai")).unwrap().as_deref(),
                Some("sk-open-1"),
                "settings file there: {settings_file_is_there}"
            );
            assert_eq!(launch.lock().layout, Layout::Current);
        }
    }

    /// A stored item the store cannot give back (it will not decrypt) is an
    /// error from every read, and a save does not write over it.
    #[test]
    fn an_item_the_store_cannot_read_is_an_error_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::leaked();
        let source = Arc::new(Mutex::new(Some(memory)));
        store_over(dir.path(), &source)
            .put(Field::ApiKey("anthropic"), "sk-ant-1")
            .unwrap();
        let kept = memory.item(SECRETS_ACCOUNT);

        *memory.fault.lock().unwrap() = Fault::Fails("the saved secret could not be decrypted");
        let launch = store_over(dir.path(), &source);
        let failure = launch.get(Field::ApiKey("anthropic")).unwrap_err();
        assert!(
            failure.reason().contains("could not be decrypted"),
            "{failure}"
        );
        assert!(launch.put(Field::ApiKey("anthropic"), "sk-ant-2").is_err());
        assert_eq!(memory.item(SECRETS_ACCOUNT), kept);
    }

    // --- persistence ---

    #[test]
    fn a_store_that_keeps_a_value_is_durable_and_is_asked_once() {
        let store = Memory::new();
        let worked = AtomicBool::new(false);
        assert_eq!(probe(Some(&store), &worked), Persistence::Durable);
        assert!(store.accounts().is_empty(), "the probe's value is removed");
        let asked = store.calls();
        assert_eq!(asked, 3, "a save, a read and a removal");
        assert_eq!(probe(Some(&store), &worked), Persistence::Durable);
        assert_eq!(store.calls(), asked, "not asked again once it has worked");
    }

    #[test]
    fn no_store_or_one_that_does_not_keep_a_value_is_not_durable() {
        let worked = AtomicBool::new(false);
        let reason = |persistence: Persistence| match persistence {
            Persistence::SessionOnly { reason } => reason,
            Persistence::Durable => panic!("reported as durable"),
        };

        assert!(reason(probe(None, &worked)).contains("not set up yet"));

        let failing = Memory::with(Fault::Fails("the Android Keystore refused"));
        assert!(reason(probe(Some(&failing), &worked)).contains("the Android Keystore refused"));

        // What `keyring`'s mock store amounted to across launches: a save
        // that answers `Ok` for a value that is not kept.
        let forgetful = Memory::with(Fault::Forgets);
        assert!(reason(probe(Some(&forgetful), &worked)).contains("was not there to read"));

        let garbling = Memory::with(Fault::Garbles);
        assert!(reason(probe(Some(&garbling), &worked)).contains("read back changed"));
        assert!(garbling.accounts().is_empty(), "removed all the same");

        assert!(
            !worked.load(Ordering::Relaxed),
            "a failure is asked again next time"
        );
        // And a store that starts to work is found to.
        *failing.fault.lock().unwrap() = Fault::None;
        assert_eq!(probe(Some(&failing), &worked), Persistence::Durable);
    }

    /// The probe writes to its account, so it must be no slot's name, and
    /// not the account the desktop probe reads and never writes.
    #[test]
    fn the_round_trip_account_collides_with_no_slot() {
        for fixed in [
            SECRETS_ACCOUNT,
            PROBE_ACCOUNT,
            ACTIVE_PROVIDER_ACCOUNT,
            CHATGPT_REFRESH_TOKEN_ACCOUNT,
        ] {
            assert_ne!(ROUND_TRIP_ACCOUNT, fixed);
        }
        // Every other slot is `<provider id><suffix>`, an MCP server's own
        // key included (`<slot>_api_key`).
        for suffix in [
            account_for(""),
            base_url_account_for(""),
            model_account_for(""),
            effort_account_for(""),
        ] {
            assert!(suffix.starts_with('_'), "{suffix}");
            assert!(!ROUND_TRIP_ACCOUNT.ends_with(&suffix), "{suffix}");
        }
    }

    // --- the text a bridge carries ---

    #[test]
    fn a_secret_crosses_a_text_bridge_whole() {
        for value in [
            &b""[..],
            b"sk-ant-1",
            br#"{"anthropic_api_key":"sk-ant-1"}"#,
            &[0, 255, 254, 10, 13],
        ] {
            let text = secret_to_text(value);
            assert!(text.is_ascii());
            assert_eq!(secret_from_text(&text).unwrap(), value);
        }
        // What `android.util.Base64` with `NO_WRAP` writes: padded, standard.
        assert_eq!(secret_to_text(b"sk-1"), "c2stMQ==");
    }

    #[test]
    fn text_that_is_not_a_secret_is_an_error_that_quotes_none_of_it() {
        for text in ["sk-ant-secret!", "c2stMQ", "c2st MQ=="] {
            let failure = secret_from_text(text).unwrap_err();
            assert!(!failure.contains("sk-ant"), "{failure}");
            assert!(!failure.contains("c2st"), "{failure}");
        }
    }
}
