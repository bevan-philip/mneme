use std::fmt::Write as _;

use anki::card::CardId;
use anki::collection::Collection;
use anki::notes::Note;
use anki::notetype::Notetype;
use anki::search::SortMode;
use anki::text::strip_html;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

/// One row of a `cards list` listing.
struct CardSummary {
    id: i64,
    deck_name: String,
    notetype_name: String,
    question: String,
}

/// List every card matching `search` (Anki's own search syntax, e.g.
/// `deck:Spanish tag:verb`; an empty string matches everything) as a
/// tab-separated table.
pub fn list_cards(col: &mut Collection, search: &str) -> Result<String> {
    let card_ids = col.search_cards(search, SortMode::NoOrder)?;
    let mut summaries = Vec::with_capacity(card_ids.len());
    for cid in card_ids {
        let card = col
            .storage
            .get_card(cid)?
            .context("card disappeared mid-listing")?;
        let deck = col
            .get_deck(card.deck_id())?
            .context("card references a missing deck")?;
        let note = col
            .storage
            .get_note(card.note_id())?
            .context("card references a missing note")?;
        let notetype = col
            .get_notetype(note.notetype_id)?
            .context("note references a missing notetype")?;
        let rendered = col.render_existing_card(cid, false, false)?;
        summaries.push(CardSummary {
            id: cid.0,
            deck_name: deck.name.human_name(),
            notetype_name: notetype.name.clone(),
            question: strip_html(&rendered.question()).into_owned(),
        });
    }
    Ok(format_card_list(&summaries))
}

fn format_card_list(summaries: &[CardSummary]) -> String {
    let mut out = String::new();
    for s in summaries {
        let preview = s.question.lines().next().unwrap_or("");
        writeln!(
            out,
            "{}\t{}\t{}\t{}",
            s.id,
            sanitize_tsv_field(&s.deck_name),
            sanitize_tsv_field(&s.notetype_name),
            sanitize_tsv_field(preview),
        )
        .unwrap();
    }
    out
}

/// Replace a literal tab with a space so every row of the tab-separated
/// `cards list` output has exactly four columns, regardless of what the
/// deck/notetype name or rendered question happens to contain.
fn sanitize_tsv_field(field: &str) -> String {
    field.replace('\t', " ")
}

/// Dump the note behind `card_id` as an editable text file (see
/// `render_note_file`).
pub fn show(col: &mut Collection, card_id: i64) -> Result<String> {
    let card = col
        .storage
        .get_card(CardId(card_id))?
        .with_context(|| format!("no such card: {card_id}"))?;
    let note = col
        .storage
        .get_note(card.note_id())?
        .context("card references a missing note")?;
    let notetype = col
        .get_notetype(note.notetype_id)?
        .context("note references a missing notetype")?;
    Ok(render_note_file(&note, &notetype))
}

/// Parse `file_text` (as produced by `show`) and apply it to the note behind
/// `card_id`. Editing one card of a note with several card templates edits
/// all of that note's cards, same as Anki's own editor.
pub fn edit(col: &mut Collection, card_id: i64, file_text: &str) -> Result<()> {
    let card = col
        .storage
        .get_card(CardId(card_id))?
        .with_context(|| format!("no such card: {card_id}"))?;
    let mut note = col
        .storage
        .get_note(card.note_id())?
        .context("card references a missing note")?;
    let notetype = col
        .get_notetype(note.notetype_id)?
        .context("note references a missing notetype")?;

    let parsed = parse_note_file(file_text)?;
    apply_note_file(&mut note, &notetype, &parsed)?;
    col.update_note(&mut note)?;
    Ok(())
}

/// Create a new note of `notetype_name` in `deck_name`, generating its
/// cards. `fields` must name every field of the notetype, in the notetype's
/// own order. Returns the new note id and the number of cards generated.
pub fn add(
    col: &mut Collection,
    notetype_name: &str,
    deck_name: &str,
    fields: &[(String, String)],
    tags: &[String],
) -> Result<(i64, usize)> {
    let notetype = col
        .get_notetype_by_name(notetype_name)?
        .with_context(|| format!("no such notetype: {notetype_name}"))?;
    let deck_id = col
        .get_deck_id(deck_name)?
        .with_context(|| format!("no such deck: {deck_name} (see `decks list`)"))?;

    let mut note = Note::new(&notetype);
    set_fields(&mut note, &notetype, fields)?;
    note.tags = tags.to_vec();

    let card_count = col.add_note(&mut note, deck_id)?.output;
    Ok((note.id.0, card_count))
}

/// The plain-text format used to dump a note's fields to a file and read
/// them back: a `# notetype:`/`# tags:` header for context, then one
/// `## FieldName` section per field holding that field's raw (HTML)
/// content. Kept close to Markdown so it's comfortable to open in a text
/// editor or browser-based HTML preview.
fn render_note_file(note: &Note, notetype: &Notetype) -> String {
    let mut out = String::new();
    writeln!(out, "# notetype: {}", notetype.name).unwrap();
    writeln!(out, "# tags: {}", note.tags.join(" ")).unwrap();
    out.push('\n');
    for (field, value) in notetype.fields.iter().zip(note.fields()) {
        writeln!(out, "## {}", field.name).unwrap();
        out.push_str(&escape_field(value));
        // A fixed two-newline boundary (not "however many the value
        // already ends with"), so parsing can strip exactly that much back
        // off and recover the value's own trailing newlines untouched.
        out.push('\n');
        out.push('\n');
    }
    out
}

/// The line that separates notes in a multi-note `--input` file (see
/// `parse_note_files_for_add`). Chosen to not collide with the existing
/// `# notetype:`/`## FieldName` header syntax.
const NOTE_DELIMITER: &str = "---";

/// Escape a line that would otherwise be misread as a `## FieldName`
/// section boundary, a note delimiter, or as an escaped line, when the
/// field's raw content contains it verbatim.
pub(crate) fn escape_field(value: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            if line.starts_with('\\') || line.starts_with("## ") || line == NOTE_DELIMITER {
                format!("\\{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Undo `escape_field`, one already-split line at a time (parsing already
/// works line by line).
pub(crate) fn unescape_field_line(line: &str) -> &str {
    line.strip_prefix('\\').unwrap_or(line)
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct ParsedNoteFile {
    pub(crate) notetype: Option<String>,
    pub(crate) deck: Option<String>,
    pub(crate) note_id: Option<i64>,
    pub(crate) tags: Option<Vec<String>>,
    pub(crate) fields: Vec<(String, String)>,
}

pub(crate) fn parse_note_file(text: &str) -> Result<ParsedNoteFile> {
    let mut parsed = ParsedNoteFile::default();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.peek() {
        if line.starts_with("## ") {
            break;
        }
        if let Some(rest) = line.strip_prefix("# notetype:") {
            parsed.notetype = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# deck:") {
            parsed.deck = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# nid:") {
            let rest = rest.trim();
            parsed.note_id = Some(
                rest.parse()
                    .with_context(|| format!("`# nid:` is not a note id: {rest}"))?,
            );
        } else if let Some(rest) = line.strip_prefix("# tags:") {
            parsed.tags = Some(rest.split_whitespace().map(str::to_string).collect());
        }
        lines.next();
    }

    let mut current: Option<(String, String)> = None;
    for line in lines {
        if let Some(name) = line.strip_prefix("## ") {
            if let Some((name, content)) = current.take() {
                parsed.fields.push((name, trim_field(content)));
            }
            current = Some((name.trim().to_string(), String::new()));
        } else if let Some((_, content)) = current.as_mut() {
            content.push_str(unescape_field_line(line));
            content.push('\n');
        } else {
            bail!("expected a `## FieldName` heading, found: {line}");
        }
    }
    if let Some((name, content)) = current.take() {
        parsed.fields.push((name, trim_field(content)));
    }

    if parsed.fields.is_empty() {
        bail!("no `## FieldName` sections found");
    }

    Ok(parsed)
}

/// Strip exactly the two-newline boundary `render_note_file` always adds
/// after a field's value, leaving any trailing newlines that were actually
/// part of the value untouched. A hand-edited file that omits the trailing
/// blank line (e.g. no blank line before EOF) just has fewer than two to
/// strip, which is fine.
pub(crate) fn trim_field(mut content: String) -> String {
    for _ in 0..2 {
        if content.ends_with('\n') {
            content.pop();
        } else {
            break;
        }
    }
    content
}

fn apply_note_file(note: &mut Note, notetype: &Notetype, parsed: &ParsedNoteFile) -> Result<()> {
    if let Some(expected) = &parsed.notetype
        && expected != &notetype.name
    {
        bail!(
            "file is for notetype `{expected}`, but this note uses `{}`",
            notetype.name
        );
    }

    set_fields(note, notetype, &parsed.fields)?;

    if let Some(tags) = &parsed.tags {
        note.tags = tags.clone();
    }

    Ok(())
}

/// Set every field of `note` from `fields`, which must name exactly the
/// notetype's own fields, in the notetype's own order: field content is
/// free-form HTML, so matching by position rather than reordering by name
/// avoids guessing at intent when a name is misspelled.
pub(crate) fn set_fields(
    note: &mut Note,
    notetype: &Notetype,
    fields: &[(String, String)],
) -> Result<()> {
    let expected: Vec<&str> = notetype.fields.iter().map(|f| f.name.as_str()).collect();
    let given: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
    if given != expected {
        bail!(
            "fields {given:?} don't match notetype `{}` fields {expected:?}",
            notetype.name
        );
    }

    for (idx, (_, value)) in fields.iter().enumerate() {
        note.set_field(idx, value.clone())?;
    }

    Ok(())
}

/// Split a `--input` file into one chunk per note, on a line that's
/// exactly `NOTE_DELIMITER` *and* immediately preceded by a blank line (or
/// is the first line of the file) — the same position every `## ` section
/// header occupies, per `render_note_file`'s fixed two-newline field
/// boundary. `escape_field` unconditionally escapes a field body line
/// that's exactly `---` (regardless of what surrounds it), so a file dumped
/// after this delimiter existed is never ambiguous. But a file dumped
/// *before* it existed could contain a legitimate, unescaped `---` line as
/// part of a field's own content; requiring the blank line keeps such a
/// line, so long as it isn't already isolated as its own paragraph, from
/// being misread as a note boundary instead of being preserved as content.
/// A file with no recognized delimiter line at all is just one chunk (the
/// whole file), so single-note files parse exactly as before.
fn split_note_blocks(text: &str) -> Vec<&str> {
    note_block_spans(text)
        .into_iter()
        .map(|span| &text[span])
        .collect()
}

/// The byte ranges of `split_note_blocks`'s blocks within `text`, excluding
/// the delimiter lines themselves. Kept as ranges into the original text
/// (rather than re-joined lines) so `cards sync` can write note ids back
/// into the file at a block's exact position, leaving every other byte —
/// including CRLF line endings — untouched.
pub(crate) fn note_block_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut block_start = 0;
    let mut offset = 0;
    let mut previous_blank = true;
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line
            .strip_suffix('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .unwrap_or(raw_line);
        if line == NOTE_DELIMITER && previous_blank {
            spans.push(block_start..offset);
            block_start = offset + raw_line.len();
        }
        previous_blank = line.trim().is_empty();
        offset += raw_line.len();
    }
    spans.push(block_start..text.len());
    spans
}

/// Parse a `--input` file (see `show`/`render_note_file`) holding one or
/// more notes for `cards add`: returns, per note, its optional notetype
/// header (for the caller to cross-check against `--notetype`), its tags,
/// and its fields. Errors (rather than silently returning zero notes) if
/// the file has no notes at all, e.g. it's empty or whitespace-only —
/// `parse_note_file` already refused that for the single-note case, and an
/// empty batch silently "succeeding" would hide a bad file the same way
/// aborting a batch partway through would.
#[allow(clippy::type_complexity)]
pub fn parse_note_files_for_add(
    text: &str,
) -> Result<Vec<(Option<String>, Vec<String>, Vec<(String, String)>)>> {
    let blocks: Vec<&str> = split_note_blocks(text)
        .into_iter()
        .filter(|block| !block.trim().is_empty())
        .collect();
    if blocks.is_empty() {
        bail!("no `## FieldName` sections found");
    }
    blocks
        .into_iter()
        .map(|block| {
            let parsed = parse_note_file(block)?;
            Ok((
                parsed.notetype,
                parsed.tags.unwrap_or_default(),
                parsed.fields,
            ))
        })
        .collect()
}

/// The outcome of adding one note from a `cards add --input` batch.
#[derive(Debug)]
pub enum NoteOutcome {
    Added { note_id: i64, card_count: usize },
    Failed { error: String },
}

/// Add every note in `parsed_notes` (as produced by
/// `parse_note_files_for_add`) to `deck_name`, all under `notetype_name`.
///
/// Every note's fields are validated against the notetype's own fields
/// (and, if present, its `# notetype:` header against `notetype_name`)
/// before anything is added to the collection: a typo'd field name or a
/// mismatched header fails the whole batch, adding none of it, the same
/// typo-safety guarantee single-note `add` already gives. Past that point
/// each note is added independently and its own success or failure (e.g. a
/// Cloze note missing its cloze deletion, which rslib only rejects at
/// add-time) is reported without blocking the rest of the batch.
#[allow(clippy::type_complexity)]
pub fn add_many(
    col: &mut Collection,
    notetype_name: &str,
    deck_name: &str,
    parsed_notes: &[(Option<String>, Vec<String>, Vec<(String, String)>)],
) -> Result<Vec<NoteOutcome>> {
    let notetype = col
        .get_notetype_by_name(notetype_name)?
        .with_context(|| format!("no such notetype: {notetype_name}"))?;
    let deck_id = col
        .get_deck_id(deck_name)?
        .with_context(|| format!("no such deck: {deck_name} (see `decks list`)"))?;

    let mut prepared = Vec::with_capacity(parsed_notes.len());
    for (idx, (file_notetype, tags, fields)) in parsed_notes.iter().enumerate() {
        if let Some(file_notetype) = file_notetype
            && file_notetype != notetype_name
        {
            bail!(
                "note {}: file is for notetype `{file_notetype}`, but --notetype was `{notetype_name}`",
                idx + 1
            );
        }
        let mut note = Note::new(&notetype);
        set_fields(&mut note, &notetype, fields).with_context(|| format!("note {}", idx + 1))?;
        note.tags = tags.clone();
        prepared.push(note);
    }

    let mut outcomes = Vec::with_capacity(prepared.len());
    for mut note in prepared {
        match col.add_note(&mut note, deck_id) {
            Ok(output) => outcomes.push(NoteOutcome::Added {
                note_id: note.id.0,
                card_count: output.output,
            }),
            Err(err) => outcomes.push(NoteOutcome::Failed {
                error: err.to_string(),
            }),
        }
    }
    Ok(outcomes)
}

/// Parse repeated `--field NAME=VALUE` arguments into ordered pairs.
pub fn parse_field_args(args: &[String]) -> Result<Vec<(String, String)>> {
    args.iter()
        .map(|arg| {
            let (name, value) = arg
                .split_once('=')
                .with_context(|| format!("expected NAME=VALUE, got `{arg}`"))?;
            Ok((name.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;

    use super::*;

    fn basic_deck_and_notetype(col: &mut Collection) -> (String, String) {
        col.get_or_create_normal_deck("Default").unwrap();
        let notetype = col.get_notetype_by_name("Basic").unwrap().unwrap();
        ("Default".to_string(), notetype.name.clone())
    }

    #[test]
    fn round_trips_a_note_through_show_and_edit() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![
            ("Front".to_string(), "What is 2+2?".to_string()),
            ("Back".to_string(), "<b>4</b>".to_string()),
        ];
        let (_note_id, card_count) = add(&mut col, &notetype, &deck, &fields, &[]).unwrap();
        assert_eq!(card_count, 1);

        let card_id = col
            .search_cards("", SortMode::NoOrder)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let dumped = show(&mut col, card_id.0).unwrap();
        assert!(dumped.contains("# notetype: Basic"));
        assert!(dumped.contains("## Front\nWhat is 2+2?\n"));
        assert!(dumped.contains("## Back\n<b>4</b>\n"));

        let edited = dumped.replace("<b>4</b>", "four");
        edit(&mut col, card_id.0, &edited).unwrap();

        let redumped = show(&mut col, card_id.0).unwrap();
        assert!(redumped.contains("## Back\nfour\n"));
    }

    #[test]
    fn preserves_trailing_newlines_through_show_and_edit() {
        use anki::notes::NoteId;

        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![
            ("Front".to_string(), "Q".to_string()),
            ("Back".to_string(), "line1\nline2\n".to_string()),
        ];
        let (note_id, _) = add(&mut col, &notetype, &deck, &fields, &[]).unwrap();

        let card_id = col
            .search_cards("", SortMode::NoOrder)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        // Re-applying an unedited dump must be a no-op on the stored field.
        let dumped = show(&mut col, card_id.0).unwrap();
        edit(&mut col, card_id.0, &dumped).unwrap();

        let note = col.storage.get_note(NoteId(note_id)).unwrap().unwrap();
        assert_eq!(note.fields()[1], "line1\nline2\n");
    }

    #[test]
    fn preserves_field_lines_that_look_like_section_headers() {
        use anki::notes::NoteId;

        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let back = "before\n## Not a heading\n\\## Also not one\nafter";
        let fields = vec![
            ("Front".to_string(), "Q".to_string()),
            ("Back".to_string(), back.to_string()),
        ];
        let (note_id, _) = add(&mut col, &notetype, &deck, &fields, &[]).unwrap();

        let card_id = col
            .search_cards("", SortMode::NoOrder)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let dumped = show(&mut col, card_id.0).unwrap();
        edit(&mut col, card_id.0, &dumped).unwrap();

        let note = col.storage.get_note(NoteId(note_id)).unwrap().unwrap();
        assert_eq!(note.fields()[1], back);
    }

    #[test]
    fn rslib_normalizes_carriage_returns_out_of_fields() {
        // rslib's own field normalization (`notes::invalid_char_for_field`)
        // strips every ASCII control character except `\n`/`\t` on every
        // `add_note`/`update_note`, so a stored field can never contain a
        // `\r` regardless of what cards.rs's own parsing does with one.
        // Pinned here so nobody "fixes" the line-splitting in
        // `parse_note_file` to chase a CRLF that can't reach storage in the
        // first place.
        use anki::notes::NoteId;

        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![
            ("Front".to_string(), "Q".to_string()),
            ("Back".to_string(), "line1\r\nline2".to_string()),
        ];
        let (note_id, _) = add(&mut col, &notetype, &deck, &fields, &[]).unwrap();

        let note = col.storage.get_note(NoteId(note_id)).unwrap().unwrap();
        assert_eq!(note.fields()[1], "line1\nline2");
    }

    #[test]
    fn sanitizes_tabs_in_card_list_columns() {
        let summaries = vec![CardSummary {
            id: 1,
            deck_name: "Deck".to_string(),
            notetype_name: "Basic".to_string(),
            question: "a\tb".to_string(),
        }];
        let listing = format_card_list(&summaries);
        assert_eq!(listing, "1\tDeck\tBasic\ta b\n");
    }

    #[test]
    fn rejects_a_field_mismatch() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![("Front".to_string(), "only one field".to_string())];
        let err = add(&mut col, &notetype, &deck, &fields, &[]).unwrap_err();
        assert!(err.to_string().contains("don't match notetype"));
    }

    #[test]
    fn parses_multiple_notes_separated_by_delimiter() {
        let text = "## Front\nhola\n\n## Back\nhello\n\n---\n## Front\nadios\n\n## Back\nbye\n";
        let parsed = parse_note_files_for_add(text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].2,
            vec![
                ("Front".to_string(), "hola".to_string()),
                ("Back".to_string(), "hello".to_string()),
            ]
        );
        assert_eq!(
            parsed[1].2,
            vec![
                ("Front".to_string(), "adios".to_string()),
                ("Back".to_string(), "bye".to_string()),
            ]
        );
    }

    #[test]
    fn single_note_file_without_delimiter_still_parses_as_one_note() {
        let text = "# notetype: Basic\n\n## Front\nhola\n\n## Back\nhello\n";
        let parsed = parse_note_files_for_add(text).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0.as_deref(), Some("Basic"));
    }

    #[test]
    fn rejects_an_empty_input_file() {
        let err = parse_note_files_for_add("").unwrap_err();
        assert!(err.to_string().contains("no `## FieldName` sections found"));

        let err = parse_note_files_for_add("   \n\n  \n").unwrap_err();
        assert!(err.to_string().contains("no `## FieldName` sections found"));
    }

    #[test]
    fn a_delimiter_line_not_preceded_by_a_blank_line_is_kept_as_field_content() {
        // Simulates a file dumped by a `cards show` from before this
        // delimiter existed, where a field's own `---` line was never
        // escaped: since it isn't isolated on its own blank-preceded line,
        // it must be read back as ordinary content, not a note boundary.
        let text = "## Front\nQ\n\n## Back\nline1\n---\nline2\n";
        let parsed = parse_note_files_for_add(text).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].2[1].1, "line1\n---\nline2");
    }

    #[test]
    fn field_value_containing_the_delimiter_round_trips() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![
            ("Front".to_string(), "Q".to_string()),
            ("Back".to_string(), "before\n---\nafter".to_string()),
        ];
        let (_note_id, _) = add(&mut col, &notetype, &deck, &fields, &[]).unwrap();

        let card_id = col
            .search_cards("", SortMode::NoOrder)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let dumped = show(&mut col, card_id.0).unwrap();

        // A dumped field body line matching the note delimiter must have
        // been escaped, or a multi-note parse would misread it as a
        // boundary.
        let parsed = parse_note_files_for_add(&dumped).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].2[1].1, "before\n---\nafter");
    }

    #[test]
    fn batch_with_one_invalid_notetype_header_fails_without_adding_any() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);

        let text = "## Front\nhola\n\n## Back\nhello\n\n---\n# notetype: NoSuchNotetype\n\n## Front\nadios\n\n## Back\nbye\n";
        let parsed = parse_note_files_for_add(text).unwrap();
        let err = add_many(&mut col, &notetype, &deck, &parsed).unwrap_err();
        assert!(err.to_string().contains("note 2"));

        let remaining = col.search_cards("", SortMode::NoOrder).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn adds_a_batch_of_notes_and_reports_each_ones_id() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);

        let text = "## Front\nhola\n\n## Back\nhello\n\n---\n## Front\nadios\n\n## Back\nbye\n";
        let parsed = parse_note_files_for_add(text).unwrap();
        let outcomes = add_many(&mut col, &notetype, &deck, &parsed).unwrap();

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            assert!(matches!(outcome, NoteOutcome::Added { card_count: 1, .. }));
        }

        let remaining = col.search_cards("", SortMode::NoOrder).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn parses_tags_and_fields() {
        let text = "# notetype: Basic\n# tags: verb spanish\n\n## Front\nhola\n\n## Back\nhello\n";
        let parsed = parse_note_file(text).unwrap();
        assert_eq!(parsed.notetype.as_deref(), Some("Basic"));
        assert_eq!(
            parsed.tags,
            Some(vec!["verb".to_string(), "spanish".to_string()])
        );
        assert_eq!(
            parsed.fields,
            vec![
                ("Front".to_string(), "hola".to_string()),
                ("Back".to_string(), "hello".to_string()),
            ]
        );
    }
}
