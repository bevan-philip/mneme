use anki::collection::Collection;
use anki_proto::decks::DeckTreeNode;
use anyhow::Result;

/// Fetch the deck tree via rslib's own scheduler-aware search, and render it.
pub fn list_decks(col: &mut Collection) -> Result<String> {
    let tree = col.deck_tree(None)?;
    Ok(render_deck_tree(&tree))
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
}
