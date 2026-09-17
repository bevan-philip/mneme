use std::fs;
use std::path::Path;

use anyhow::Result;
use anyhow::bail;

/// Profile directories under `anki2_root` that contain a collection file.
pub fn discover_profiles(anki2_root: &Path) -> Result<Vec<String>> {
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
            profiles.push(entry.file_name().to_string_lossy().into_owned());
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
pub fn resolve_profile(anki2_root: &Path, requested: Option<&str>) -> Result<String> {
    if let Some(name) = requested {
        return Ok(name.to_string());
    }
    match discover_profiles(anki2_root)?.as_slice() {
        [] => bail!(
            "no Anki profiles found under {}; pass --profile explicitly",
            anki2_root.display()
        ),
        [only] => Ok(only.clone()),
        many => bail!(
            "multiple Anki profiles found ({}); pass --profile to pick one",
            many.join(", ")
        ),
    }
}
