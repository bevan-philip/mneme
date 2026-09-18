use std::path::PathBuf;

use anki::collection::Collection;
use anyhow::Context;
use anyhow::Result;

/// A directory per profile, since rslib's own backup thinning treats every
/// file in the directory it's given as one collection's history: sharing a
/// directory across profiles would let backing up one profile prune
/// another's recovery points.
fn backup_root(profile: &str) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("could not determine the platform cache directory")?;
    Ok(base.join("anki-cli").join("backups").join(profile))
}

/// Back up the (already open) collection through rslib's own backup
/// machinery, rather than copying the live `.anki2` file: rslib checkpoints
/// the WAL before reading, so the snapshot reflects committed state instead
/// of a possibly half-written file, and it writes a real `.colpkg` with
/// collision-resistant, self-thinning naming. Callers should do this before
/// any mutating command runs, so a bug in this CLI can't destroy a user's
/// real data.
pub fn backup_collection(col: &mut Collection, profile: &str) -> Result<PathBuf> {
    let dir = backup_root(profile)?;
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
