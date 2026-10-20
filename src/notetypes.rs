use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use anki::collection::Collection;
use anki::notetype::Notetype;
use anki::notetype::NotetypeId;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use unicode_normalization::UnicodeNormalization;

use crate::cards::escape_field;
use crate::cards::split_sections;
use crate::cards::trim_field;

/// List every notetype's name and id, one per line, sorted by name for
/// deterministic output (rslib doesn't guarantee an order of its own).
pub fn list_notetypes(col: &mut Collection) -> Result<String> {
    let mut notetypes = col.get_all_notetypes()?;
    notetypes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(render_notetype_list(&notetypes))
}

fn render_notetype_list(notetypes: &[Arc<Notetype>]) -> String {
    let mut out = String::new();
    for nt in notetypes {
        writeln!(out, "{}\t{}", nt.name, nt.id.0).unwrap();
    }
    out
}

fn notetype_by_name(col: &mut Collection, name: &str) -> Result<Arc<Notetype>> {
    col.get_notetype_by_name(name)?
        .with_context(|| format!("no such notetype: {name} (see `notetypes list`)"))
}

/// Show a notetype's field names, in the exact order `cards add --field`/the
/// note-file format expect, plus its template names. With `full`, also each
/// template's front/back HTML and the notetype's CSS, so the styling Anki
/// holds can be read without opening Anki.
pub fn show_notetype(col: &mut Collection, name: &str, full: bool) -> Result<String> {
    let notetype = notetype_by_name(col, name)?;
    Ok(if full {
        render_notetype_file(&notetype)
    } else {
        render_notetype(&notetype, false)
    })
}

/// The `show --full` output that `edit`/`add --input` read back.
fn render_notetype_file(notetype: &Notetype) -> String {
    let mut out = render_notetype(notetype, true);
    out.push_str(&render_notetype_styling(notetype));
    // The last section's fixed two-newline boundary.
    out.push('\n');
    out
}

/// The `--full` sections: each template's front and back, then the CSS.
/// Every section ends with the same fixed two-newline boundary
/// `cards::render_note_file` uses, and its body is escaped the same way, so
/// `parse_notetype_file` can recover each value (bar a `\r` before a line
/// break, which `str::lines` drops).
fn render_notetype_styling(notetype: &Notetype) -> String {
    let mut out = String::new();
    for template in &notetype.templates {
        push_section(
            &mut out,
            &format!("template: {} (front)", template.name),
            &template.config.q_format,
        );
        push_section(
            &mut out,
            &format!("template: {} (back)", template.name),
            &template.config.a_format,
        );
    }
    push_section(&mut out, "css", &notetype.config.css);
    out
}

fn push_section(out: &mut String, header: &str, value: &str) {
    writeln!(out, "\n## {header}").unwrap();
    out.push_str(&escape_field(value));
    out.push('\n');
}

/// The name lists are only escaped with `full`, for `parse_notetype_file`;
/// otherwise they're the exact names `cards add --field` takes.
fn render_notetype(notetype: &Notetype, full: bool) -> String {
    let list = |names: Vec<&str>| {
        let names = names.join("\n");
        if full { escape_field(&names) } else { names }
    };
    let fields = list(notetype.fields.iter().map(|f| f.name.as_str()).collect());
    let templates = list(notetype.templates.iter().map(|t| t.name.as_str()).collect());
    let mut out = String::new();
    writeln!(out, "# notetype: {}", notetype.name).unwrap();
    out.push('\n');
    writeln!(out, "## fields").unwrap();
    writeln!(out, "{fields}").unwrap();
    out.push('\n');
    writeln!(out, "## templates").unwrap();
    writeln!(out, "{templates}").unwrap();
    out
}

#[derive(Debug)]
pub(crate) struct ParsedNotetypeFile {
    notetype: Option<String>,
    fields: Vec<String>,
    templates: Vec<String>,
    sides: BTreeMap<String, Sides>,
    css: Value,
}

#[derive(Default, Debug)]
struct Sides {
    front: Option<Value>,
    back: Option<Value>,
}

impl Sides {
    fn get(&self, front: bool) -> Option<&Value> {
        if front { &self.front } else { &self.back }.as_ref()
    }
}

/// A template side or the CSS. `exact` is false only for the file's last
/// section when its trailing blank lines were trimmed (as many editors do on
/// save): the value's own trailing newlines can't be told apart from the
/// missing boundary then, so `resolve` keeps the ones already stored.
#[derive(Debug)]
struct Value {
    text: String,
    exact: bool,
}

impl Value {
    /// `stored` itself if this reads the same as `show` printed it
    /// (`shown`), so an untouched value keeps its exact bytes; else this.
    fn pick(&self, shown: &str, stored: &str) -> String {
        if self.resolve(shown) == shown {
            stored.to_string()
        } else {
            self.resolve(stored)
        }
    }

    fn resolve(&self, current: &str) -> String {
        if self.exact {
            return self.text.clone();
        }
        let ending = &current[current.trim_end_matches('\n').len()..];
        format!("{}{ending}", self.text.trim_end_matches('\n'))
    }
}

/// Parse `notetypes show --full` output.
pub(crate) fn parse_notetype_file(text: &str) -> Result<ParsedNotetypeFile> {
    let (preamble, sections) = split_sections(text.lines());

    let mut notetype = None;
    for line in preamble {
        if let Some(rest) = line.strip_prefix("# notetype:") {
            // Not trimmed: rslib allows leading/trailing spaces in a name.
            notetype = Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }

    let (mut fields, mut templates, mut css) = (None, None, None);
    let mut sides: BTreeMap<String, Sides> = BTreeMap::new();
    let last = sections.len().saturating_sub(1);
    for (i, (header, content)) in sections.into_iter().enumerate() {
        let header = header.trim_end();
        let value = Value {
            exact: i != last || content.ends_with("\n\n"),
            text: trim_field(content),
        };
        let slot = match header {
            "fields" => &mut fields,
            "templates" => &mut templates,
            "css" => &mut css,
            _ => {
                let (name, front) = parse_template_header(header)?;
                let entry = sides.entry(name.to_string()).or_default();
                if front {
                    &mut entry.front
                } else {
                    &mut entry.back
                }
            }
        };
        if slot.replace(value).is_some() {
            bail!("duplicate `## {header}` section");
        }
    }

    let list = |value: Option<Value>, header: &str| -> Result<Vec<String>> {
        let value = value.with_context(|| format!("missing `## {header}` section"))?;
        Ok(value.text.split('\n').map(str::to_string).collect())
    };
    Ok(ParsedNotetypeFile {
        notetype,
        fields: list(fields, "fields")?,
        templates: list(templates, "templates")?,
        sides,
        css: css.context("missing `## css` section")?,
    })
}

/// Split `template: NAME (front)`/`(back)` into the name and whether it's
/// the front.
fn parse_template_header(header: &str) -> Result<(&str, bool)> {
    let rest = header
        .strip_prefix("template: ")
        .with_context(|| format!("unknown section `## {header}`"))?;
    if let Some(name) = rest.strip_suffix(" (front)") {
        Ok((name, true))
    } else if let Some(name) = rest.strip_suffix(" (back)") {
        Ok((name, false))
    } else {
        bail!("`## {header}` must end in ` (front)` or ` (back)`")
    }
}

/// Apply a parsed file's templates and CSS to `notetype`. The file's field
/// and template lists must match the notetype's exactly: adding, removing,
/// renaming or reordering either is a schema change in Anki, which forces a
/// one-way full sync, so it's left to Anki desktop.
fn apply_notetype_file(notetype: &mut Notetype, parsed: &ParsedNotetypeFile) -> Result<()> {
    // The file is checked against what `show --full` printed, read back the
    // same way, rather than against the stored values `str::lines` might not
    // reproduce exactly.
    let shown = parse_notetype_file(&render_notetype_file(notetype))?;
    if let Some(expected) = &parsed.notetype
        && parsed.notetype != shown.notetype
    {
        bail!("file is for notetype `{expected}`, not `{}`", notetype.name);
    }
    if parsed.fields != shown.fields {
        bail!(
            "fields {:?} don't match notetype `{}` fields {:?}; changing fields \
             forces a full sync, so do it in Anki desktop",
            parsed.fields,
            notetype.name,
            shown.fields
        );
    }
    if parsed.templates != shown.templates {
        bail!(
            "templates {:?} don't match notetype `{}` templates {:?}; changing \
             templates forces a full sync, so do it in Anki desktop",
            parsed.templates,
            notetype.name,
            shown.templates
        );
    }
    if let Some(name) = parsed
        .sides
        .keys()
        .find(|name| !shown.templates.contains(name))
    {
        bail!("`## template: {name}` isn't in the `## templates` list");
    }

    for (template, name) in notetype.templates.iter_mut().zip(&shown.templates) {
        let side = |front: bool| -> Result<(&Value, &Value)> {
            let which = if front { "front" } else { "back" };
            let file = parsed.sides.get(name).and_then(|s| s.get(front));
            let file = file.with_context(|| format!("missing `## template: {name} ({which})`"))?;
            // `render_notetype_file` always writes both sides.
            Ok((file, shown.sides[name].get(front).unwrap()))
        };
        let (file, as_shown) = side(true)?;
        template.config.q_format = file.pick(&as_shown.text, &template.config.q_format);
        let (file, as_shown) = side(false)?;
        template.config.a_format = file.pick(&as_shown.text, &template.config.a_format);
    }
    notetype.config.css = parsed.css.pick(&shown.css.text, &notetype.config.css);
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditOutcome {
    Updated,
    Unchanged,
}

/// Replace a notetype's templates and CSS with those in a `show --full`
/// file. rslib's `update_notetype` only marks the schema modified when field
/// or template ordinals change (or the sort field moves), none of which
/// `apply_notetype_file` allows, so this syncs normally.
pub fn edit_notetype(col: &mut Collection, name: &str, text: &str) -> Result<EditOutcome> {
    let parsed = parse_notetype_file(text)?;
    let original = notetype_by_name(col, name)?;
    let mut notetype = (*original).clone();
    apply_notetype_file(&mut notetype, &parsed)?;
    if notetype == *original {
        return Ok(EditOutcome::Unchanged);
    }
    col.update_notetype(&mut notetype, false)?;
    Ok(EditOutcome::Updated)
}

/// Clone notetype `from` as `name`, the way desktop's Manage Note Types →
/// Add → Clone does (`models.copy`: same everything, fresh id, no
/// `original_id`), optionally applying a `show --full` file's templates and
/// CSS first. Adding a notetype isn't a schema change: it travels in a
/// normal sync like any other changed object. Returns the new id and name.
pub fn add_notetype(
    col: &mut Collection,
    name: &str,
    from: &str,
    text: Option<&str>,
) -> Result<(i64, String)> {
    let parsed = text.map(parse_notetype_file).transpose()?;
    // rslib strips `"` and NFC-normalizes the name, then silently appends
    // `+` until it's unique, so check the name it would actually store.
    let name: String = name.replace('"', "").nfc().collect();
    if col.get_notetype_by_name(&name)?.is_some() {
        bail!("notetype `{name}` already exists");
    }
    let source = notetype_by_name(col, from)?;
    let mut notetype = (*source).clone();
    if let Some(parsed) = &parsed {
        apply_notetype_file(&mut notetype, parsed)?;
    }
    notetype.id = NotetypeId(0);
    notetype.name = name.clone();
    notetype.config.original_id = None;
    col.add_notetype(&mut notetype, false)
        .with_context(|| format!("failed to create notetype `{name}`"))?;
    Ok((notetype.id.0, notetype.name))
}

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;

    use super::*;

    /// Pins the exact rslib field order for a stock notetype against a real
    /// in-memory collection: `cards add`/the note-file format depend on this
    /// order, and rslib has no semver guarantees on it. `Basic (and reversed
    /// card)` is used because it has two templates sharing one field set,
    /// which exercises both the fields and templates listings.
    #[test]
    fn shows_fields_in_declared_order_and_all_templates() {
        let mut col = CollectionBuilder::default().build().unwrap();

        let shown = show_notetype(&mut col, "Basic (and reversed card)", false).unwrap();

        assert_eq!(
            shown,
            "# notetype: Basic (and reversed card)\n\
             \n\
             ## fields\n\
             Front\n\
             Back\n\
             \n\
             ## templates\n\
             Card 1\n\
             Card 2\n"
        );
    }

    #[test]
    fn full_show_includes_each_template_and_the_css() {
        let mut col = CollectionBuilder::default().build().unwrap();

        let shown = show_notetype(&mut col, "Basic (and reversed card)", true).unwrap();

        assert!(shown.contains(
            "## template: Card 1 (front)
{{Front}}
"
        ));
        assert!(shown.contains(
            "## template: Card 2 (front)
{{Back}}
"
        ));
        assert!(shown.contains(
            "## css
.card {"
        ));
    }

    /// Sets awkward names and template/CSS content directly through rslib
    /// (lines that look like section headers or escapes, a `---`, lone `\r`s
    /// mid-line and at the end of a value, CRLF, trailing newlines,
    /// a name ending in a space), then checks `show --full` → `edit` is a
    /// no-op.
    #[test]
    fn full_show_round_trips_exactly_through_edit() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let mut nt = (*col
            .get_notetype_by_name("Basic (and reversed card)")
            .unwrap()
            .unwrap())
        .clone();
        nt.name = "Vocab ".into();
        nt.templates[0].config.q_format =
            "{{Front}}\n## not a header\n\\backslash\n---\n<br>\r<br>\n".into();
        nt.templates[0].config.a_format = "{{FrontSide}}\r\n<hr>\r\n{{Back}}\r".into();
        nt.fields[1].name = "\\Back".into();
        nt.templates[1].name = "## Card 2".into();
        nt.templates[1].config.a_format = "{{FrontSide}}\n\n## also not\n\n\n".into();
        nt.config.css = ".card {}\n## css\r".into();
        col.update_notetype(&mut nt, false).unwrap();
        let stored = col.get_notetype_by_name(&nt.name).unwrap().unwrap();
        assert_eq!(stored.name, "Vocab ");

        let shown = show_notetype(&mut col, &nt.name, true).unwrap();
        assert!(shown.contains("\n\\## not a header\n\\\\backslash\n\\---\n<br>\r<br>\n"));
        assert!(shown.contains("## fields\nFront\n\\\\Back\n"));
        assert!(shown.contains("## templates\nCard 1\n\\## Card 2\n"));
        // Without `--full`, names are printed as they are.
        let plain = show_notetype(&mut col, &nt.name, false).unwrap();
        assert!(plain.contains("## fields\nFront\n\\Back\n"));
        assert!(plain.contains("## templates\nCard 1\n## Card 2\n"));
        let mut reparsed = (*stored).clone();
        apply_notetype_file(&mut reparsed, &parse_notetype_file(&shown).unwrap()).unwrap();
        assert_eq!(reparsed, *stored);

        assert_eq!(
            edit_notetype(&mut col, &nt.name, &shown).unwrap(),
            EditOutcome::Unchanged
        );
    }

    /// A CSS stored with CRLF keeps it when an unrelated template is edited.
    #[test]
    fn stored_crlf_survives_an_unrelated_edit() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let mut nt = (*col.get_notetype_by_name("Basic").unwrap().unwrap()).clone();
        nt.config.css = ".card {\r\n}\r\n".into();
        col.update_notetype(&mut nt, false).unwrap();
        let shown = show_notetype(&mut col, "Basic", true).unwrap();

        let edited = shown.replace("{{Front}}\n", "<b>{{Front}}</b>\n");
        assert_eq!(
            edit_notetype(&mut col, "Basic", &edited).unwrap(),
            EditOutcome::Updated
        );
        let stored = col.get_notetype_by_name("Basic").unwrap().unwrap();
        assert_eq!(stored.config.css, ".card {\r\n}\r\n");
    }

    /// Editors commonly trim trailing blank lines on save, which eats the
    /// last section's boundary; the CSS's own trailing newline survives.
    #[test]
    fn edit_keeps_the_css_ending_when_trailing_blank_lines_are_trimmed() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let mut nt = (*col.get_notetype_by_name("Basic").unwrap().unwrap()).clone();
        nt.config.css = ".card {}\n".into();
        col.update_notetype(&mut nt, false).unwrap();
        let shown = show_notetype(&mut col, "Basic", true).unwrap();
        let trimmed = shown.trim_end_matches('\n');

        for text in [format!("{trimmed}\n"), trimmed.to_string()] {
            assert_eq!(
                edit_notetype(&mut col, "Basic", &text).unwrap(),
                EditOutcome::Unchanged
            );
        }
        let edited = trimmed.replace(".card {}", ".card { color: red; }");
        edit_notetype(&mut col, "Basic", &edited).unwrap();
        let stored = col.get_notetype_by_name("Basic").unwrap().unwrap();
        assert_eq!(stored.config.css, ".card { color: red; }\n");
    }

    #[test]
    fn edit_applies_templates_and_css() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let shown = show_notetype(&mut col, "Basic", true).unwrap();
        let edited = shown
            .replace("{{Front}}\n", "<b>{{Front}}</b>\n")
            .replace(".card {", ".card {\n    color: red;");

        assert_eq!(
            edit_notetype(&mut col, "Basic", &edited).unwrap(),
            EditOutcome::Updated
        );
        assert_eq!(show_notetype(&mut col, "Basic", true).unwrap(), edited);
    }

    #[test]
    fn edit_accepts_a_file_an_editor_converted_to_crlf() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let shown = show_notetype(&mut col, "Basic", true).unwrap();

        assert_eq!(
            edit_notetype(&mut col, "Basic", &shown.replace('\n', "\r\n")).unwrap(),
            EditOutcome::Unchanged
        );
    }

    /// rslib marks the schema modified (forcing a one-way full sync) when
    /// field or template ordinals change or the sort field moves, but not for
    /// template HTML/CSS edits or for adding a notetype. Pinned via the
    /// collection's schema timestamp, with a template reorder as the positive
    /// control showing the probe would catch a schema change.
    #[test]
    fn template_and_css_edits_and_clones_leave_the_schema_unmodified() {
        let dir = tempfile::tempdir().unwrap();
        let mut col = CollectionBuilder::new(dir.path().join("collection.anki2"))
            .build()
            .unwrap();
        let schema = |col: &Collection| col.sync_meta().unwrap().schema;
        let before = schema(&col);
        std::thread::sleep(std::time::Duration::from_millis(5));

        let shown = show_notetype(&mut col, "Basic (and reversed card)", true).unwrap();
        let edited = shown
            .replace(
                "{{Back}}\n\n## template: Card 2 (back)",
                "<i>{{Back}}</i>\n\n## template: Card 2 (back)",
            )
            .replace(".card {", ".card {\n    color: red;");
        assert_eq!(
            edit_notetype(&mut col, "Basic (and reversed card)", &edited).unwrap(),
            EditOutcome::Updated
        );
        add_notetype(&mut col, "Clone", "Basic", None).unwrap();
        assert_eq!(schema(&col), before);

        let mut nt = (*col
            .get_notetype_by_name("Basic (and reversed card)")
            .unwrap()
            .unwrap())
        .clone();
        nt.templates.swap(0, 1);
        col.update_notetype(&mut nt, false).unwrap();
        assert_ne!(schema(&col), before);
    }

    #[test]
    fn edit_rejects_field_or_template_list_changes() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let shown = show_notetype(&mut col, "Basic (and reversed card)", true).unwrap();

        let reordered = shown.replace("Front\nBack\n", "Back\nFront\n");
        let err = edit_notetype(&mut col, "Basic (and reversed card)", &reordered).unwrap_err();
        assert!(err.to_string().contains("full sync"), "{err}");

        let renamed = shown.replace("Card 2", "Reverse");
        let err = edit_notetype(&mut col, "Basic (and reversed card)", &renamed).unwrap_err();
        assert!(err.to_string().contains("full sync"), "{err}");

        let unknown = shown.replace("## template: Card 2 (back)", "## template: Card 3 (back)");
        let err = edit_notetype(&mut col, "Basic (and reversed card)", &unknown).unwrap_err();
        assert!(err.to_string().contains("isn't in the"), "{err}");

        let err = edit_notetype(&mut col, "Basic", &shown).unwrap_err();
        assert!(err.to_string().contains("file is for notetype"), "{err}");
    }

    #[test]
    fn add_clones_a_notetype_and_applies_an_input_file() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let shown = show_notetype(&mut col, "Basic", true).unwrap();
        let edited = shown.replace(".card {", ".card {\n    color: red;");

        let (id, name) = add_notetype(&mut col, "Styled", "Basic", Some(&edited)).unwrap();

        assert_eq!(name, "Styled");
        let clone = col.get_notetype_by_name("Styled").unwrap().unwrap();
        assert_eq!(clone.id.0, id);
        assert_eq!(
            show_notetype(&mut col, "Styled", true).unwrap(),
            edited.replace("# notetype: Basic", "# notetype: Styled")
        );
        // The source is untouched.
        assert_eq!(show_notetype(&mut col, "Basic", true).unwrap(), shown);
    }

    #[test]
    fn add_rejects_an_existing_name_or_unknown_source() {
        let mut col = CollectionBuilder::default().build().unwrap();

        let err = add_notetype(&mut col, "Cloze", "Basic", None).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");

        // Names rslib would only make unique by appending `+`.
        add_notetype(&mut col, "Caf\u{e9}", "Basic", None).unwrap();
        for name in ["Cloze\"", "Cafe\u{301}"] {
            let err = add_notetype(&mut col, name, "Basic", None).unwrap_err();
            assert!(err.to_string().contains("already exists"), "{err}");
        }

        let err = add_notetype(&mut col, "New", "No Such Notetype", None).unwrap_err();
        assert!(err.to_string().contains("no such notetype"), "{err}");
    }

    #[test]
    fn errors_on_unknown_notetype() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let err = show_notetype(&mut col, "No Such Notetype", false).unwrap_err();
        assert!(err.to_string().contains("no such notetype"));
    }

    #[test]
    fn lists_notetype_names_sorted() {
        let mut col = CollectionBuilder::default().build().unwrap();

        let listing = list_notetypes(&mut col).unwrap();

        let names: Vec<&str> = listing
            .lines()
            .map(|l| l.split('\t').next().unwrap())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert!(names.contains(&"Basic"));
    }
}
