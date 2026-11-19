use std::path::Path;

use anki::collection::Collection;
use anki::notetype::Notetype;
use anyhow::Result;
use anyhow::bail;

/// One card, rendered as the two pages Anki's previewer shows for it.
pub struct RenderedCard {
    /// File-name stem: the card id for a stored card, `note<N>-card<M>` for
    /// one a note file would generate.
    pub name: String,
    pub front: String,
    pub back: String,
    /// Anything the preview can't show the way Anki would.
    pub warnings: Vec<String>,
}

/// Render stored cards, through `templates` (a notetype with edited
/// templates/CSS, from `notetypes::preview_notetype`) instead of the stored
/// notetype when given. `media_dir` is the collection's media folder.
pub fn render_cards(
    _col: &mut Collection,
    _card_ids: &[i64],
    _templates: Option<&Notetype>,
    _media_dir: &Path,
) -> Result<Vec<RenderedCard>> {
    bail!("rendering stored cards is not implemented yet")
}

/// Render every card the notes in a note file would have, without adding
/// or changing anything.
pub fn render_notes(
    _col: &mut Collection,
    _text: &str,
    _templates: Option<&Notetype>,
    _media_dir: &Path,
) -> Result<Vec<RenderedCard>> {
    bail!("rendering a note file is not implemented yet")
}
