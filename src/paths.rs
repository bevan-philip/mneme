use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

/// The platform root that holds per-profile Anki folders, e.g.
/// `~/.local/share/Anki2` on Linux, `~/Library/Application Support/Anki2` on
/// macOS, `%APPDATA%\Anki2` on Windows.
pub fn default_anki2_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Anki2"))
}

/// Path to a profile's collection file, given the Anki2 root and profile
/// name. Takes the profile name as a raw `OsStr`, not `str`: it's a
/// directory name we got from `read_dir`, and lossy-converting it before
/// rejoining it to a path could target a directory that doesn't exist.
pub fn collection_path(anki2_root: &Path, profile: &OsStr) -> PathBuf {
    anki2_root.join(profile).join("collection.anki2")
}

/// A short, stable fingerprint (FNV-1a) of a path's on-disk byte
/// representation. Used to tell apart same-named profiles under different
/// `--anki2-root`s without depending on the path already existing on disk.
///
/// Hashes the platform's lossless representation of the path rather than
/// `to_string_lossy()`'s output: on Unix two different non-UTF-8 paths can
/// both collapse to the same replacement-character string and so collide.
pub fn fingerprint(path: &Path) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut absorb = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    };
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for byte in path.as_os_str().as_bytes() {
            absorb(*byte);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            for byte in unit.to_le_bytes() {
                absorb(byte);
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        for byte in path.to_string_lossy().as_bytes() {
            absorb(*byte);
        }
    }
    format!("{hash:016x}")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn distinct_non_utf8_paths_have_distinct_fingerprints() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // Both invalid UTF-8, and both collapse to the same string under
        // `to_string_lossy` (a single U+FFFD), but they're different paths.
        let a = Path::new(OsStr::from_bytes(b"/root/\xff"));
        let b = Path::new(OsStr::from_bytes(b"/root/\xfe"));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        assert_ne!(fingerprint(a), fingerprint(b));
    }
}
