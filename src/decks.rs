use anki::collection::Collection;
use anki_proto::decks::DeckTreeNode;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

/// Fetch the deck tree via rslib's own scheduler-aware search, and render it.
pub fn list_decks(col: &mut Collection) -> Result<String> {
    let tree = col.deck_tree(None)?;
    Ok(render_deck_tree(&tree))
}

/// Create a new deck named `name` (which may be `::`-nested, e.g.
/// "Spanish::Vocab::Week3"), erroring if a deck with that exact name already
/// exists rather than silently returning the existing one:
/// `get_or_create_normal_deck` is happy to do the latter, but that's the
/// wrong default for a command whose entire point is "make a new deck", per
/// this codebase's usual typo-safety stance. Any missing parent segments of
/// a nested name are created automatically by rslib's own
/// `match_or_create_parents`, the same as Anki's deck picker.
pub fn add_deck(col: &mut Collection, name: &str) -> Result<i64> {
    if col.get_deck_id(name)?.is_some() {
        bail!("deck `{name}` already exists");
    }
    let deck = col
        .get_or_create_normal_deck(name)
        .with_context(|| format!("failed to create deck `{name}`"))?;
    Ok(deck.id.0)
}

/// Turn a deck tree into an indented listing, one deck per line.
fn render_deck_tree(node: &DeckTreeNode) -> String {
    let mut out = String::new();
    for child in &node.children {
        render_node(child, &mut out);
    }
    out
}

fn render_node(node: &DeckTreeNode, out: &mut String) {
    out.push_str(&"  ".repeat(node.level.saturating_sub(1) as usize));
    out.push_str(&node.name);
    out.push('\n');
    for child in &node.children {
        render_node(child, out);
    }
}

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;

    use super::*;

    /// Pins rslib's deck-tree level numbering, verified against a real
    /// (in-memory) collection: the synthetic root is level 0, so real
    /// top-level decks are level 1 and get no indentation. Worth guarding
    /// explicitly since rslib has no semver guarantees on this.
    #[test]
    fn indents_nested_decks_by_level() {
        let mut col = CollectionBuilder::default().build().unwrap();
        col.get_or_create_normal_deck("Parent").unwrap();
        col.get_or_create_normal_deck("Parent::Child").unwrap();
        col.get_or_create_normal_deck("Other").unwrap();

        let listing = list_decks(&mut col).unwrap();

        assert_eq!(listing, "Other\nParent\n  Child\n");
    }

    #[test]
    fn creates_nested_deck_with_missing_parents() {
        let mut col = CollectionBuilder::default().build().unwrap();

        add_deck(&mut col, "Parent::Child").unwrap();

        let listing = list_decks(&mut col).unwrap();
        assert_eq!(listing, "Parent\n  Child\n");
    }

    #[test]
    fn rejects_creating_a_deck_that_already_exists() {
        let mut col = CollectionBuilder::default().build().unwrap();
        col.get_or_create_normal_deck("Existing").unwrap();

        let err = add_deck(&mut col, "Existing").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }
}
