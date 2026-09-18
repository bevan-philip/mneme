use std::path::Path;
use std::path::PathBuf;

use anki::collection::Collection;
use anyhow::Context;
use anyhow::Result;

use crate::paths;

/// A directory per (anki2_root, profile) pair, since rslib's own backup
/// thinning treats every file in the directory it's given as one
/// collection's history: sharing a directory across profiles — or across
/// same-named profiles under different `--anki2-root`s — would let backing
/// up one collection prune another's recovery points.
fn backup_root(anki2_root: &Path, profile: &str) -> Result<PathBuf> {
    let base = dirs::cache_dir().context("could not determine the platform cache directory")?;
    let absolute_root = std::path::absolute(anki2_root)
        .with_context(|| format!("failed to resolve {}", anki2_root.display()))?;
    let namespace = format!("{}-{profile}", paths::fingerprint(&absolute_root));
    Ok(base.join("anki-cli").join("backups").join(namespace))
}

/// Back up the (already open) collection through rslib's own backup
/// machinery, rather than copying the live `.anki2` file: rslib checkpoints
/// the WAL before reading, so the snapshot reflects committed state instead
/// of a possibly half-written file, and it writes a real `.colpkg` with
/// collision-resistant, self-thinning naming. Callers should do this before
/// any mutating command runs, so a bug in this CLI can't destroy a user's
/// real data.
pub fn backup_collection(
    col: &mut Collection,
    anki2_root: &Path,
    profile: &str,
) -> Result<PathBuf> {
    let dir = backup_root(anki2_root, profile)?;
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
