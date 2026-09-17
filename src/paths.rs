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
