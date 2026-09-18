use std::path::Path;
use std::path::PathBuf;

use anki::collection::Collection;
use anyhow::Context;
use anyhow::Result;

use crate::paths;

/// A directory unique to this exact collection file, since rslib's own
/// backup thinning treats every file in the directory it's given as one
/// collection's retention history: sharing a directory across collections
/// (different profiles, or same-named profiles under different
/// `--anki2-root`s) would let backing up one collection prune another's
/// recovery points. Named from a lossless fingerprint of the full,
/// lexically-absolute collection path; the profile-name suffix is only for
/// browsability; it doesn't need to be collision-resistant itself.
fn backup_root(collection_path: &Path) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("could not determine the platform cache directory")?;
    let absolute = std::path::absolute(collection_path)
        .with_context(|| format!("failed to resolve {}", collection_path.display()))?;
    let label = collection_path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let namespace = format!("{}-{label}", paths::fingerprint(&absolute));
    Ok(base.join("anki-cli").join("backups").join(namespace))
}

/// Back up the (already open) collection through rslib's own backup
/// machinery, rather than copying the live `.anki2` file: rslib checkpoints
/// the WAL before reading, so the snapshot reflects committed state instead
/// of a possibly half-written file, and it writes a real `.colpkg` with
/// collision-resistant, self-thinning naming. Callers should do this before
/// any mutating command runs, so a bug in this CLI can't destroy a user's
/// real data.
pub fn backup_collection(col: &mut Collection, collection_path: &Path) -> Result<PathBuf> {
    let dir = backup_root(collection_path)?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create backup directory {}", dir.display()))?;
    col.maybe_backup(dir.clone(), true)
        .context("failed to back up collection")?
        .context("rslib declined to back up an unchanged collection")?
        .join()
        .map_err(|_| anyhow::anyhow!("backup thread panicked"))?
        .context("failed to back up collection")?;
    Ok(dir)
}
