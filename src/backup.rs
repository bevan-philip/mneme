use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;

/// The backup file's name for a given unix timestamp.
fn backup_file_name(unix_secs: u64) -> String {
    format!("collection-{unix_secs}.anki2")
}

fn backup_root() -> Result<PathBuf> {
    let base = dirs::cache_dir().context("could not determine the platform cache directory")?;
    Ok(base.join("anki-cli").join("backups"))
}

/// Copy the collection file to a scratch backup location. Callers should do
/// this before any mutating command runs, so a bug in this CLI can't destroy
/// a user's real data.
pub fn backup_collection(collection_path: &Path) -> Result<PathBuf> {
    let dir = backup_root()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create backup directory {}", dir.display()))?;
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the unix epoch")?
        .as_secs();
    let dest = dir.join(backup_file_name(unix_secs));
    fs::copy(collection_path, &dest).with_context(|| {
        format!(
            "failed to back up {} to {}",
            collection_path.display(),
            dest.display()
        )
    })?;
    Ok(dest)
}
