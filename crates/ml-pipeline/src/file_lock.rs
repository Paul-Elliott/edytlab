//! An exclusive lock on a file, between processes: what the downloader and
//! the stem cache hold on their `.lock` files.
//!
//! `std::fs::File::lock` and `try_lock` are the lock everywhere but
//! Android. The standard library builds them for a list of systems, and in
//! the pinned toolchain that list has Linux and not Android, so on a phone
//! they answer "try_lock() not supported" and every model download failed
//! with that message before it asked for a byte. Android's kernel has
//! `flock` like any Linux, so there this module calls it itself.
//!
//! Keep callers on these three functions. A `file.lock()` written straight
//! against the standard library compiles for Android and fails only on a
//! device.

use std::fs::{File, TryLockError};
use std::io;

/// Take the lock, waiting for whoever holds it.
pub fn lock(file: &File) -> io::Result<()> {
    imp::lock(file)
}

/// Take the lock if it is free. [`TryLockError::WouldBlock`] when another
/// handle holds it.
pub fn try_lock(file: &File) -> Result<(), TryLockError> {
    imp::try_lock(file)
}

/// Give the lock back. Closing the file does the same.
pub fn unlock(file: &File) -> io::Result<()> {
    imp::unlock(file)
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::{io, File, TryLockError};

    pub(super) fn lock(file: &File) -> io::Result<()> {
        file.lock()
    }

    pub(super) fn try_lock(file: &File) -> Result<(), TryLockError> {
        file.try_lock()
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        file.unlock()
    }
}

#[cfg(target_os = "android")]
mod imp {
    pub(super) use super::flock::{lock, try_lock, unlock};
}

/// The lock by `flock(2)`. Android's, and built for the tests of every unix
/// so the code a phone runs is run somewhere before it reaches one.
#[cfg(any(target_os = "android", all(test, unix)))]
mod flock {
    use super::{io, File, TryLockError};
    use std::os::fd::AsRawFd;

    fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
        // SAFETY: the descriptor is open for as long as `file` is borrowed,
        // and `flock` reads nothing but the two integers it is given.
        if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn lock(file: &File) -> io::Result<()> {
        flock(file, libc::LOCK_EX)
    }

    pub(super) fn try_lock(file: &File) -> Result<(), TryLockError> {
        match flock(file, libc::LOCK_EX | libc::LOCK_NB) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Err(TryLockError::WouldBlock),
            Err(e) => Err(TryLockError::Error(e)),
        }
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        flock(file, libc::LOCK_UN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    fn open(path: &std::path::Path) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap()
    }

    /// What every platform's lock must do: a second handle is refused while
    /// the first holds it, and let in once it is given back.
    fn holds_and_releases(
        try_lock: fn(&File) -> Result<(), TryLockError>,
        unlock: fn(&File) -> io::Result<()>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.lock");
        let first = open(&path);
        let second = open(&path);

        try_lock(&first).expect("nobody holds it yet");
        assert!(matches!(try_lock(&second), Err(TryLockError::WouldBlock)));

        unlock(&first).unwrap();
        try_lock(&second).expect("free once it is given back");
        assert!(matches!(try_lock(&first), Err(TryLockError::WouldBlock)));

        unlock(&second).unwrap();
        try_lock(&first).expect("free again");
    }

    #[test]
    fn a_held_lock_refuses_a_second_handle_until_it_is_released() {
        holds_and_releases(try_lock, unlock);
    }

    /// The same, through the code an Android build uses.
    #[cfg(unix)]
    #[test]
    fn the_flock_path_android_uses_holds_and_releases_the_same_way() {
        holds_and_releases(flock::try_lock, flock::unlock);

        // Closing the holder frees the lock too: what `ArtifactLock` and the
        // stem cache rely on when a run ends without unlocking.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closed.lock");
        let (holder, other) = (open(&path), open(&path));
        flock::try_lock(&holder).unwrap();
        drop(holder);
        flock::try_lock(&other).expect("free once the holder is closed");
    }

    /// The waiting lock comes back once the holder lets go.
    #[cfg(unix)]
    #[test]
    fn the_waiting_flock_returns_when_the_holder_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stems.lock");
        let holder = open(&path);
        flock::lock(&holder).unwrap();

        let waiter = open(&path);
        let waiting = std::thread::spawn(move || {
            flock::lock(&waiter).unwrap();
            waiter
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !waiting.is_finished(),
            "it must wait while the lock is held"
        );
        flock::unlock(&holder).unwrap();
        let waiter = waiting.join().unwrap();
        assert!(matches!(
            flock::try_lock(&holder),
            Err(TryLockError::WouldBlock)
        ));
        drop(waiter);
    }
}
