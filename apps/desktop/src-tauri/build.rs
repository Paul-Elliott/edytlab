use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    embed_bundled_resources();
    tauri_build::build()
}

/// Write `$OUT_DIR/bundled_resources.rs`: every file the installers carry
/// as a resource (`bundle.resources` in `tauri.conf.json`), by the path it
/// has inside the resource folder, with its bytes.
///
/// The Android app is the reader (`src/resources.rs`). An APK holds these
/// files as assets, which have no path `std::fs` can open or list, so that
/// build carries them in the library and writes them to the app's storage
/// at first launch. The file is generated for every target (it is a few
/// lines) and compiled into Android builds and tests only.
///
/// Keep the three folders below the same as `bundle.resources`.
fn embed_bundled_resources() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for (source, bundled_as) in [
        ("resources/templates", "templates"),
        ("resources/skills", "bundled-skills"),
    ] {
        let dir = manifest_dir.join(source);
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        for file in files {
            let name = file
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            entries.push((format!("{bundled_as}/{name}"), file));
        }
    }
    let notices = manifest_dir.join("licenses/THIRD_PARTY_NOTICES.txt");
    println!("cargo:rerun-if-changed={}", notices.display());
    entries.push(("licenses/THIRD_PARTY_NOTICES.txt".to_string(), notices));

    let mut source = String::from(
        "/// `(path inside the resource folder, contents)`; made by `build.rs`.\n\
         pub(crate) static BUNDLED_RESOURCES: &[(&str, &[u8])] = &[\n",
    );
    for (bundled_as, file) in &entries {
        writeln!(
            source,
            "    ({bundled_as:?}, include_bytes!({:?})),",
            path_literal(file)
        )
        .expect("writing to a String");
    }
    source.push_str("];\n");
    std::fs::write(out_dir.join("bundled_resources.rs"), source)
        .expect("writing bundled_resources.rs");
}

/// `path` as `include_bytes!` wants it: forward slashes work on every host.
fn path_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
