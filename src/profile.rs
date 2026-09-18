use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use anyhow::Result;
use anyhow::bail;

/// Profile directories under `anki2_root` that contain a collection file.
/// Returned as `OsString`, not `String`: these are real directory names
/// from `read_dir`, and lossy-converting them here would let a non-UTF-8
/// name get rejoined into a path that doesn't actually exist.
pub fn discover_profiles(anki2_root: &Path) -> Result<Vec<OsString>> {
    let mut profiles = Vec::new();
    if !anki2_root.is_dir() {
        return Ok(profiles);
    }
    for entry in fs::read_dir(anki2_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if entry.path().join("collection.anki2").is_file() {
            profiles.push(entry.file_name());
        }
    }
    profiles.sort();
    Ok(profiles)
}

/// Resolve which profile to use: the explicit `--profile`, or the sole
/// profile found on disk.
///
/// `prefs21.db` records Anki desktop's own default profile, but as a pickled
/// Python value, so we don't parse it here; ambiguity is instead resolved by
/// asking for `--profile` explicitly. See AGENTS.md for follow-up notes.
pub fn resolve_profile(anki2_root: &Path, requested: Option<&OsStr>) -> Result<OsString> {
    if let Some(name) = requested {
        if !anki2_root.join(name).join("collection.anki2").is_file() {
            // Collection opening always creates a missing file, so without
            // this check a typo'd --profile would silently create and
            // operate on a fresh, empty collection instead of erroring.
            bail!(
                "no Anki profile named \"{}\" found under {}",
                name.to_string_lossy(),
                anki2_root.display()
            );
        }
        return Ok(name.to_os_string());
    }
    match discover_profiles(anki2_root)?.as_slice() {
        [] => bail!(
            "no Anki profiles found under {}; pass --profile explicitly",
            anki2_root.display()
        ),
        [only] => Ok(only.clone()),
        many => bail!(
            "multiple Anki profiles found ({}); pass --profile to pick one",
            many.iter()
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
