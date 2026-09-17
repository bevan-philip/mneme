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
