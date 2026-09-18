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
            s.id, s.deck_name, s.notetype_name, preview
        )
        .unwrap();
    }
    out
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

/// Escape a line that would otherwise be misread as a `## FieldName`
/// section boundary, or as an escaped line, when the field's raw content
/// contains it verbatim.
fn escape_field(value: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            if line.starts_with('\\') || line.starts_with("## ") {
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
fn unescape_field_line(line: &str) -> &str {
    line.strip_prefix('\\').unwrap_or(line)
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ParsedNoteFile {
    notetype: Option<String>,
    tags: Option<Vec<String>>,
    fields: Vec<(String, String)>,
}

fn parse_note_file(text: &str) -> Result<ParsedNoteFile> {
    let mut parsed = ParsedNoteFile::default();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.peek() {
        if line.starts_with("## ") {
            break;
        }
        if let Some(rest) = line.strip_prefix("# notetype:") {
            parsed.notetype = Some(rest.trim().to_string());
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
fn trim_field(mut content: String) -> String {
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
fn set_fields(note: &mut Note, notetype: &Notetype, fields: &[(String, String)]) -> Result<()> {
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

/// Parse a note file (see `show`/`render_note_file`) for `cards add
/// --input`: returns its optional notetype header (for the caller to
/// cross-check against `--notetype`), its tags, and its fields.
#[allow(clippy::type_complexity)]
pub fn parse_note_file_for_add(
    text: &str,
) -> Result<(Option<String>, Vec<String>, Vec<(String, String)>)> {
    let parsed = parse_note_file(text)?;
    Ok((
        parsed.notetype,
        parsed.tags.unwrap_or_default(),
        parsed.fields,
    ))
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
    fn rejects_a_field_mismatch() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let (deck, notetype) = basic_deck_and_notetype(&mut col);
        let fields = vec![("Front".to_string(), "only one field".to_string())];
        let err = add(&mut col, &notetype, &deck, &fields, &[]).unwrap_err();
        assert!(err.to_string().contains("don't match notetype"));
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
