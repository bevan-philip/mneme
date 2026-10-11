use std::collections::HashMap;
use std::sync::Arc;

use anki::collection::Collection;
use anki::decks::DeckId;
use anki::notes::Note;
use anki::notes::NoteId;
use anki::notetype::Notetype;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

use crate::cards::note_block_spans;
use crate::cards::parse_note_file;
use crate::cards::set_fields;

/// One note of a `cards sync` file, with its headers resolved: a block's own
/// `# notetype:`/`# deck:`/`# tags:` win, and anything it omits is inherited
/// from the first block, so the top of the file reads as the file's header.
/// `# nid:` is never inherited — it identifies one note.
#[derive(Debug, PartialEq, Eq)]
pub struct SyncNote {
    /// Index of this note's block in `note_block_spans`, for id write-back.
    pub block: usize,
    pub note_id: Option<i64>,
    pub notetype: String,
    pub deck: String,
    pub tags: Vec<String>,
    pub fields: Vec<(String, String)>,
}

/// Parse a `cards sync` file into notes with resolved headers. Pure.
pub fn parse_sync_file(text: &str) -> Result<Vec<SyncNote>> {
    let mut notes: Vec<SyncNote> = Vec::new();
    for (block, span) in note_block_spans(text).into_iter().enumerate() {
        let chunk = &text[span];
        if chunk.trim().is_empty() {
            continue;
        }
        let n = notes.len() + 1;
        let parsed = parse_note_file(chunk).with_context(|| format!("note {n}"))?;
        let first = notes.first();
        let notetype = parsed
            .notetype
            .or_else(|| first.map(|f| f.notetype.clone()))
            .with_context(|| format!("note {n}: no `# notetype:` header"))?;
        let deck = parsed
            .deck
            .or_else(|| first.map(|f| f.deck.clone()))
            .with_context(|| format!("note {n}: no `# deck:` header"))?;
        let tags = parsed
            .tags
            .or_else(|| first.map(|f| f.tags.clone()))
            .unwrap_or_default();
        notes.push(SyncNote {
            block,
            note_id: parsed.note_id,
            notetype,
            deck,
            tags,
            fields: parsed.fields,
        });
    }
    if notes.is_empty() {
        bail!("no `## FieldName` sections found");
    }
    Ok(notes)
}

/// The outcome of syncing one note.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Added { note_id: i64, card_count: usize },
    Updated { note_id: i64 },
    Unchanged { note_id: i64 },
    Failed { error: String },
}

/// A note that passed validation, ready to write.
enum Prepared {
    New {
        note: Note,
        deck: String,
    },
    Existing {
        note: Note,
        changed: bool,
        deck: String,
    },
}

/// Add or update every note in `notes`. Notes with a `# nid:` update that
/// note (fields, tags, and which deck its cards are in); notes without one
/// are added. Notes deleted from the file are left alone in Anki.
///
/// Everything is validated before anything is written — notetypes exist,
/// fields match, every nid exists and belongs to its notetype, every deck
/// exists (unless `create_decks`) — so a typo fails the whole file, the same
/// guarantee `cards add --input` gives. Past that, each note is written and
/// reported independently.
pub fn sync(
    col: &mut Collection,
    notes: &[SyncNote],
    create_decks: bool,
) -> Result<Vec<SyncOutcome>> {
    let mut notetypes: HashMap<&str, Arc<Notetype>> = HashMap::new();
    let mut missing_decks = Vec::new();
    let mut prepared = Vec::with_capacity(notes.len());

    for (idx, sync_note) in notes.iter().enumerate() {
        let n = idx + 1;
        let notetype = match notetypes.get(sync_note.notetype.as_str()) {
            Some(nt) => nt.clone(),
            None => {
                let nt = col
                    .get_notetype_by_name(&sync_note.notetype)?
                    .with_context(|| {
                        format!(
                            "note {n}: no such notetype: {} (see `notetypes list`)",
                            sync_note.notetype
                        )
                    })?;
                notetypes.insert(&sync_note.notetype, nt.clone());
                nt
            }
        };

        if col.get_deck_id(&sync_note.deck)?.is_none() && !missing_decks.contains(&sync_note.deck) {
            if !create_decks {
                bail!(
                    "note {n}: no such deck: {} (pass --create-deck to create it)",
                    sync_note.deck
                );
            }
            missing_decks.push(sync_note.deck.clone());
        }

        let deck = sync_note.deck.clone();
        match sync_note.note_id {
            Some(id) => {
                let original = col
                    .storage
                    .get_note(NoteId(id))?
                    .with_context(|| format!("note {n}: no such note: {id}"))?;
                if original.notetype_id != notetype.id {
                    bail!(
                        "note {n}: note {id} is not a `{}` note (changing a note's notetype isn't supported)",
                        notetype.name
                    );
                }
                let mut note = original.clone();
                set_fields(&mut note, &notetype, &sync_note.fields)
                    .with_context(|| format!("note {n}"))?;
                note.tags = sync_note.tags.clone();
                let changed =
                    note.fields() != original.fields() || !same_tags(&note.tags, &original.tags);
                prepared.push(Prepared::Existing {
                    note,
                    changed,
                    deck,
                });
            }
            None => {
                let mut note = Note::new(&notetype);
                set_fields(&mut note, &notetype, &sync_note.fields)
                    .with_context(|| format!("note {n}"))?;
                note.tags = sync_note.tags.clone();
                prepared.push(Prepared::New { note, deck });
            }
        }
    }

    for deck in &missing_decks {
        col.get_or_create_normal_deck(deck)?;
    }

    let mut outcomes = Vec::with_capacity(prepared.len());
    for item in prepared {
        let outcome = match item {
            Prepared::New { mut note, deck } => (|| -> Result<SyncOutcome> {
                let deck_id = deck_id(col, &deck)?;
                let card_count = col.add_note(&mut note, deck_id)?.output;
                Ok(SyncOutcome::Added {
                    note_id: note.id.0,
                    card_count,
                })
            })(),
            Prepared::Existing {
                mut note,
                changed,
                deck,
            } => (|| -> Result<SyncOutcome> {
                if changed {
                    col.update_note(&mut note)?;
                }
                let moved = move_cards(col, note.id, deck_id(col, &deck)?)?;
                Ok(if changed || moved {
                    SyncOutcome::Updated { note_id: note.id.0 }
                } else {
                    SyncOutcome::Unchanged { note_id: note.id.0 }
                })
            })(),
        };
        outcomes.push(
            outcome.unwrap_or_else(|err: anyhow::Error| SyncOutcome::Failed {
                error: err.to_string(),
            }),
        );
    }
    Ok(outcomes)
}

/// Whether two tag lists are the same set as Anki sees it: rslib stores
/// tags sorted and matches them case-insensitively, so a file listing
/// `b a` must count as unchanged against a stored `a b`.
fn same_tags(a: &[String], b: &[String]) -> bool {
    let normalize = |tags: &[String]| {
        let mut tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        tags.sort_unstable();
        tags.dedup();
        tags
    };
    normalize(a) == normalize(b)
}

fn deck_id(col: &Collection, name: &str) -> Result<DeckId> {
    col.get_deck_id(name)?
        .with_context(|| format!("no such deck: {name}"))
}

/// Move any of `note_id`'s cards not already in `deck_id` there. Returns
/// whether anything moved.
fn move_cards(col: &mut Collection, note_id: NoteId, deck_id: DeckId) -> Result<bool> {
    let to_move: Vec<_> = col
        .storage
        .all_cards_of_note(note_id)?
        .into_iter()
        .filter(|card| card.deck_id() != deck_id)
        .map(|card| card.id())
        .collect();
    if to_move.is_empty() {
        return Ok(false);
    }
    col.set_deck(&to_move, deck_id)?;
    Ok(true)
}

/// Insert a `# nid: N` line at the start of each newly added note's block,
/// leaving every other byte of `text` untouched. Uses CRLF for the new line
/// if the file already does. Pure.
pub fn write_note_ids(text: &str, notes: &[SyncNote], outcomes: &[SyncOutcome]) -> String {
    let spans = note_block_spans(text);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut inserts: Vec<(usize, i64)> = notes
        .iter()
        .zip(outcomes)
        .filter_map(|(note, outcome)| match outcome {
            SyncOutcome::Added { note_id, .. } if note.note_id.is_none() => {
                Some((spans[note.block].start, *note_id))
            }
            _ => None,
        })
        .collect();
    inserts.sort_unstable();

    let mut out = String::with_capacity(text.len() + inserts.len() * 32);
    let mut copied = 0;
    for (at, note_id) in inserts {
        out.push_str(&text[copied..at]);
        out.push_str(&format!("# nid: {note_id}{eol}"));
        copied = at;
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;
    use anki::search::SortMode;

    use super::*;

    const TWO_NOTES: &str = "# notetype: Basic\n# deck: Spanish\n# tags: verb\n\n## Front\nhola\n\n## Back\nhello\n\n---\n## Front\nadios\n\n## Back\nbye\n";

    fn col_with_deck(name: &str) -> Collection {
        let mut col = CollectionBuilder::default().build().unwrap();
        col.get_or_create_normal_deck(name).unwrap();
        col
    }

    fn sync_text(col: &mut Collection, text: &str, create: bool) -> (Vec<SyncOutcome>, String) {
        let notes = parse_sync_file(text).unwrap();
        let outcomes = sync(col, &notes, create).unwrap();
        let written = write_note_ids(text, &notes, &outcomes);
        (outcomes, written)
    }

    #[test]
    fn later_blocks_inherit_the_first_blocks_headers() {
        let text = "# notetype: Basic\n# deck: A\n# tags: x y\n\n## Front\n1\n\n## Back\n1\n\n---\n# deck: B\n\n## Front\n2\n\n## Back\n2\n";
        let notes = parse_sync_file(text).unwrap();
        assert_eq!(notes[1].notetype, "Basic");
        assert_eq!(notes[1].deck, "B");
        assert_eq!(notes[1].tags, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(notes[1].block, 1);
    }

    #[test]
    fn requires_a_notetype_and_deck() {
        let err = parse_sync_file("# notetype: Basic\n\n## Front\nq\n\n## Back\na\n").unwrap_err();
        assert!(err.to_string().contains("no `# deck:` header"));
    }

    #[test]
    fn adds_notes_and_writes_their_ids_back() {
        let mut col = col_with_deck("Spanish");
        let (outcomes, written) = sync_text(&mut col, TWO_NOTES, false);
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, SyncOutcome::Added { card_count: 1, .. }))
        );
        assert_eq!(written.matches("# nid: ").count(), 2);
        assert!(written.starts_with("# nid: "));
        assert!(written.contains("---\n# nid: "));

        let note = col
            .storage
            .get_note(NoteId(
                parse_sync_file(&written).unwrap()[0].note_id.unwrap(),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(note.tags, vec!["verb".to_string()]);
    }

    #[test]
    fn resyncing_the_written_file_adds_nothing() {
        let mut col = col_with_deck("Spanish");
        let (_, written) = sync_text(&mut col, TWO_NOTES, false);
        let (outcomes, rewritten) = sync_text(&mut col, &written, false);
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, SyncOutcome::Unchanged { .. }))
        );
        assert_eq!(rewritten, written);
        assert_eq!(col.search_cards("", SortMode::NoOrder).unwrap().len(), 2);
    }

    #[test]
    fn tags_in_a_different_order_or_case_are_unchanged() {
        let mut col = col_with_deck("Spanish");
        let text = TWO_NOTES.replace("# tags: verb", "# tags: verb Spanish basics");
        let (_, written) = sync_text(&mut col, &text, false);
        let (outcomes, _) = sync_text(&mut col, &written, false);
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, SyncOutcome::Unchanged { .. }))
        );

        let reordered =
            written.replace("# tags: verb Spanish basics", "# tags: basics VERB spanish");
        let (outcomes, _) = sync_text(&mut col, &reordered, false);
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, SyncOutcome::Unchanged { .. }))
        );
    }

    #[test]
    fn an_edited_field_updates_the_note() {
        let mut col = col_with_deck("Spanish");
        let (_, written) = sync_text(&mut col, TWO_NOTES, false);
        let edited = written.replace("hello", "hi");
        let (outcomes, _) = sync_text(&mut col, &edited, false);
        assert!(matches!(outcomes[0], SyncOutcome::Updated { .. }));
        assert!(matches!(outcomes[1], SyncOutcome::Unchanged { .. }));

        let id = parse_sync_file(&edited).unwrap()[0].note_id.unwrap();
        let note = col.storage.get_note(NoteId(id)).unwrap().unwrap();
        assert_eq!(note.fields()[1], "hi");
    }

    #[test]
    fn a_changed_deck_header_moves_the_cards() {
        let mut col = col_with_deck("Spanish");
        col.get_or_create_normal_deck("Other").unwrap();
        let (_, written) = sync_text(&mut col, TWO_NOTES, false);
        let moved = written.replace("# deck: Spanish", "# deck: Other");
        let (outcomes, _) = sync_text(&mut col, &moved, false);
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, SyncOutcome::Updated { .. }))
        );
        assert_eq!(
            col.search_cards("deck:Other", SortMode::NoOrder)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn a_missing_deck_is_an_error_unless_creation_is_requested() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let notes = parse_sync_file(TWO_NOTES).unwrap();
        let err = sync(&mut col, &notes, false).unwrap_err();
        assert!(err.to_string().contains("--create-deck"));
        assert!(col.get_deck_id("Spanish").unwrap().is_none());

        let outcomes = sync(&mut col, &notes, true).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(col.get_deck_id("Spanish").unwrap().is_some());
    }

    #[test]
    fn an_unknown_note_id_fails_the_whole_file_before_writing() {
        let mut col = col_with_deck("Spanish");
        let text = TWO_NOTES.replace("---\n", "---\n# nid: 12345\n");
        let notes = parse_sync_file(&text).unwrap();
        let err = sync(&mut col, &notes, false).unwrap_err();
        assert!(err.to_string().contains("note 2: no such note: 12345"));
        assert!(col.search_cards("", SortMode::NoOrder).unwrap().is_empty());
    }

    #[test]
    fn a_note_of_another_notetype_fails_the_whole_file() {
        let mut col = col_with_deck("Spanish");
        let (_, written) = sync_text(&mut col, TWO_NOTES, false);
        let retyped = written.replacen(
            "# notetype: Basic",
            "# notetype: Basic (and reversed card)",
            1,
        );
        let notes = parse_sync_file(&retyped).unwrap();
        let err = sync(&mut col, &notes, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("changing a note's notetype isn't supported")
        );
    }

    #[test]
    fn write_back_preserves_crlf_and_every_other_byte() {
        let mut col = col_with_deck("Spanish");
        let crlf = TWO_NOTES.replace('\n', "\r\n");
        let (_, written) = sync_text(&mut col, &crlf, false);
        assert!(!written.replace("\r\n", "").contains('\n'));
        let without_ids: String = written
            .split_inclusive("\r\n")
            .filter(|line| !line.starts_with("# nid: "))
            .collect();
        assert_eq!(without_ids, crlf);
    }
}
