use std::path::Path;
use std::path::PathBuf;

/// The platform root that holds per-profile Anki folders, e.g.
/// `~/.local/share/Anki2` on Linux, `~/Library/Application Support/Anki2` on
/// macOS, `%APPDATA%\Anki2` on Windows.
pub fn default_anki2_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Anki2"))
}

/// Path to a profile's collection file, given the Anki2 root and profile
/// name.
pub fn collection_path(anki2_root: &Path, profile: &str) -> PathBuf {
    anki2_root.join(profile).join("collection.anki2")
}

/// A short, stable fingerprint (FNV-1a) of a path's textual form. Used to
/// tell apart same-named profiles under different `--anki2-root`s without
/// depending on the path already existing on disk.
pub fn fingerprint(path: &Path) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
