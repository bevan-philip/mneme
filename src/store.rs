use std::path::Path;

use anki::collection::Collection;
use anki::collection::CollectionBuilder;
use anki::error::AnkiError;
use anki::error::DbErrorKind;

/// Opening the collection can fail because Anki desktop already has it open
/// (a locked sqlite database); that case gets its own variant so callers can
/// show a clean message instead of rslib's raw sqlite error.
pub enum OpenError {
    Locked,
    Other(AnkiError),
}

/// Open a collection file the same way Anki desktop does. rslib has no
/// separate read-only mode, and always maintains undo history and mod-time
/// bookkeeping, so we always go through this rather than touching sqlite
/// directly.
pub fn open_collection(collection_path: &Path) -> Result<Collection, OpenError> {
    CollectionBuilder::new(collection_path)
        .with_desktop_media_paths()
        .build()
        .map_err(|err| match &err {
            AnkiError::DbError { source } if source.kind == DbErrorKind::Locked => {
                OpenError::Locked
            }
            _ => OpenError::Other(err),
        })
}
