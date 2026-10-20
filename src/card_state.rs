use anki::card::CardId;
use anki::collection::Collection;
use anki::search::JoinSearches;
use anki::search::SearchNode;
use anki::search::SortMode;
use anki::search::StateKind;
use anki_proto::scheduler::bury_or_suspend_cards_request::Mode as BuryOrSuspendMode;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

/// A scheduling-state change, applied to cards through the same rslib calls
/// Anki's browser uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    Suspend,
    Unsuspend,
    Bury,
    Unbury,
    Flag(u32),
}

/// Anki's flag names, indexed by flag number (0 clears the flag).
const FLAG_NAMES: [&str; 8] = [
    "none",
    "red",
    "orange",
    "green",
    "blue",
    "pink",
    "turquoise",
    "purple",
];

/// Parse a flag given as its number (0-7) or Anki's name for it.
pub fn parse_flag(value: &str) -> Result<u32> {
    let lower = value.trim().to_lowercase();
    if let Ok(n) = lower.parse::<u32>() {
        if (n as usize) < FLAG_NAMES.len() {
            return Ok(n);
        }
    } else if let Some(n) = FLAG_NAMES.iter().position(|name| *name == lower) {
        return Ok(n as u32);
    }
    bail!(
        "invalid flag `{value}`: expected 0-7 or one of {}",
        FLAG_NAMES.join(", ")
    )
}

/// Resolve the cards to act on: explicit ids (each must exist, so a typo
/// errors instead of silently matching nothing) or an Anki search. An empty
/// search would match the whole collection, so it's refused.
pub fn resolve_cards(
    col: &mut Collection,
    card_ids: &[i64],
    search: Option<&str>,
) -> Result<Vec<CardId>> {
    match search {
        Some(search) => {
            if search.trim().is_empty() {
                bail!("--search must not be empty (it would match every card)");
            }
            Ok(col.search_cards(search, SortMode::NoOrder)?)
        }
        None => {
            if card_ids.is_empty() {
                bail!("provide one or more card ids or --search QUERY");
            }
            for &cid in card_ids {
                col.storage
                    .get_card(CardId(cid))?
                    .with_context(|| format!("no such card: {cid}"))?;
            }
            Ok(card_ids.iter().map(|&cid| CardId(cid)).collect())
        }
    }
}

/// Apply `change` to `cards`, returning how many actually changed state.
/// Unsuspend and unbury only touch cards in that state: rslib's
/// `unbury_or_unsuspend_cards` restores any buried *or* suspended card, so
/// without the filter `unsuspend` would also unbury.
pub fn apply(col: &mut Collection, cards: &[CardId], change: StateChange) -> Result<usize> {
    Ok(match change {
        StateChange::Suspend => {
            col.bury_or_suspend_cards(cards, BuryOrSuspendMode::Suspend)?
                .output
        }
        StateChange::Bury => {
            // What Anki's browser and reviewer use for a manual bury.
            col.bury_or_suspend_cards(cards, BuryOrSuspendMode::BuryUser)?
                .output
        }
        StateChange::Unsuspend => restore(col, cards, StateKind::Suspended)?,
        StateChange::Unbury => restore(col, cards, StateKind::Buried)?,
        StateChange::Flag(flag) => col.set_card_flag(cards, flag)?.output,
    })
}

fn restore(col: &mut Collection, cards: &[CardId], state: StateKind) -> Result<usize> {
    if cards.is_empty() {
        return Ok(0);
    }
    let matching = col.search_cards(
        SearchNode::from_card_ids(cards.iter().copied()).and(state),
        SortMode::NoOrder,
    )?;
    col.unbury_or_unsuspend_cards(&matching)?;
    Ok(matching.len())
}

/// The summary line printed after a change.
pub fn describe(change: StateChange, changed: usize, total: usize) -> String {
    let verb = match change {
        StateChange::Suspend => "suspended".to_string(),
        StateChange::Unsuspend => "unsuspended".to_string(),
        StateChange::Bury => "buried".to_string(),
        StateChange::Unbury => "unburied".to_string(),
        StateChange::Flag(0) => "cleared the flag on".to_string(),
        StateChange::Flag(n) => format!("flagged {}", FLAG_NAMES[n as usize]),
    };
    let unchanged = total - changed;
    if unchanged == 0 {
        format!("{verb} {changed} card(s)")
    } else {
        format!(
            "{verb} {changed} of {total} card(s); {unchanged} already in that state or not eligible"
        )
    }
}

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;

    use super::*;

    /// Two Basic notes' cards in a fresh in-memory collection.
    fn two_cards() -> (Collection, Vec<CardId>) {
        let mut col = CollectionBuilder::default().build().unwrap();
        col.get_or_create_normal_deck("Default").unwrap();
        for front in ["one", "two"] {
            let fields = vec![
                ("Front".to_string(), front.to_string()),
                ("Back".to_string(), "back".to_string()),
            ];
            crate::cards::add(&mut col, "Basic", "Default", &fields, &[]).unwrap();
        }
        let cards = col.search_cards("", SortMode::NoOrder).unwrap();
        (col, cards)
    }

    fn count(col: &mut Collection, search: &str) -> usize {
        col.search_cards(search, SortMode::NoOrder).unwrap().len()
    }

    #[test]
    fn parses_flags_by_number_or_name() {
        assert_eq!(parse_flag("0").unwrap(), 0);
        assert_eq!(parse_flag("Red").unwrap(), 1);
        assert_eq!(parse_flag("purple").unwrap(), 7);
        assert!(parse_flag("8").is_err());
        assert!(parse_flag("magenta").is_err());
    }

    #[test]
    fn suspends_and_unsuspends() {
        let (mut col, cards) = two_cards();
        assert_eq!(
            apply(&mut col, &cards[..1], StateChange::Suspend).unwrap(),
            1
        );
        assert_eq!(apply(&mut col, &cards, StateChange::Suspend).unwrap(), 1);
        assert_eq!(count(&mut col, "is:suspended"), 2);
        assert_eq!(apply(&mut col, &cards, StateChange::Unsuspend).unwrap(), 2);
        assert_eq!(count(&mut col, "is:suspended"), 0);
    }

    #[test]
    fn unsuspend_leaves_buried_cards_buried_and_vice_versa() {
        let (mut col, cards) = two_cards();
        apply(&mut col, &cards[..1], StateChange::Suspend).unwrap();
        apply(&mut col, &cards[1..], StateChange::Bury).unwrap();

        assert_eq!(apply(&mut col, &cards, StateChange::Unsuspend).unwrap(), 1);
        assert_eq!(count(&mut col, "is:buried"), 1);

        apply(&mut col, &cards[..1], StateChange::Suspend).unwrap();
        assert_eq!(apply(&mut col, &cards, StateChange::Unbury).unwrap(), 1);
        assert_eq!(count(&mut col, "is:suspended"), 1);
        assert_eq!(count(&mut col, "is:buried"), 0);
    }

    #[test]
    fn bury_does_not_touch_suspended_cards() {
        let (mut col, cards) = two_cards();
        apply(&mut col, &cards[..1], StateChange::Suspend).unwrap();
        assert_eq!(apply(&mut col, &cards, StateChange::Bury).unwrap(), 1);
        assert_eq!(count(&mut col, "is:suspended"), 1);
    }

    #[test]
    fn flags_and_clears_flags() {
        let (mut col, cards) = two_cards();
        assert_eq!(apply(&mut col, &cards, StateChange::Flag(2)).unwrap(), 2);
        assert_eq!(count(&mut col, "flag:2"), 2);
        assert_eq!(
            apply(&mut col, &cards[..1], StateChange::Flag(0)).unwrap(),
            1
        );
        assert_eq!(count(&mut col, "flag:0"), 1);
    }

    #[test]
    fn resolves_cards_by_id_or_search() {
        let (mut col, cards) = two_cards();
        let ids: Vec<i64> = cards.iter().map(|c| c.0).collect();
        assert_eq!(resolve_cards(&mut col, &ids, None).unwrap().len(), 2);
        assert!(resolve_cards(&mut col, &[ids[0], 12345], None).is_err());
        assert!(resolve_cards(&mut col, &[], None).is_err());
        assert_eq!(
            resolve_cards(&mut col, &[], Some("front:one"))
                .unwrap()
                .len(),
            1
        );
        assert!(resolve_cards(&mut col, &[], Some("  ")).is_err());
    }
}
