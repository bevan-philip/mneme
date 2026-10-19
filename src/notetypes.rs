use std::fmt::Write as _;

use anki::collection::Collection;
use anki::notetype::Notetype;
use anki::notetype::NotetypeId;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

use crate::cards::escape_field;
use crate::cards::trim_field;
use crate::cards::unescape_field_line;

/// List every notetype's name and id, one per line, sorted by name for
/// deterministic output (rslib doesn't guarantee an order of its own).
pub fn list_notetypes(col: &mut Collection) -> Result<String> {
    let mut notetypes = col.get_all_notetypes()?;
    notetypes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(render_notetype_list(&notetypes))
}

fn render_notetype_list(notetypes: &[std::sync::Arc<Notetype>]) -> String {
    let mut out = String::new();
    for nt in notetypes {
        writeln!(out, "{}\t{}", nt.name, nt.id.0).unwrap();
    }
    out
}

/// Show a notetype's field names, in the exact order `cards add --field`/the
/// note-file format expect, plus its template names. With `full`, also each
/// template's front/back HTML and the notetype's CSS, so the styling Anki
/// holds can be read without opening Anki.
pub fn show_notetype(col: &mut Collection, name: &str, full: bool) -> Result<String> {
    let notetype = col
        .get_notetype_by_name(name)?
        .with_context(|| format!("no such notetype: {name} (see `notetypes list`)"))?;
    let mut out = render_notetype(&notetype);
    if full {
        out.push_str(&render_notetype_styling(&notetype));
        // The last section's fixed two-newline boundary.
        out.push('\n');
    }
    Ok(out)
}

/// The `--full` sections: each template's front and back, then the CSS.
/// Every section ends with the same fixed two-newline boundary
/// `cards::render_note_file` uses, and its body is escaped the same way, so
/// `parse_notetype_file` can recover each value exactly.
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

fn render_notetype(notetype: &Notetype) -> String {
    let fields: Vec<&str> = notetype.fields.iter().map(|f| f.name.as_str()).collect();
    let templates: Vec<&str> = notetype.templates.iter().map(|t| t.name.as_str()).collect();
    let mut out = String::new();
    writeln!(out, "# notetype: {}", notetype.name).unwrap();
    out.push('\n');
    writeln!(out, "## fields").unwrap();
    writeln!(out, "{}", escape_field(&fields.join("\n"))).unwrap();
    out.push('\n');
    writeln!(out, "## templates").unwrap();
    writeln!(out, "{}", escape_field(&templates.join("\n"))).unwrap();
    out
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct ParsedNotetypeFile {
    notetype: Option<String>,
    fields: Vec<String>,
    templates: Vec<String>,
    /// Template name -> (front, back), in file order.
    sides: Vec<(String, Option<String>, Option<String>)>,
    css: String,
}

/// Parse `notetypes show --full` output. Unlike note fields (which rslib
/// strips control characters from), a template or the CSS can legitimately
/// hold a `\r`, so lines are split on `\n` alone; only a file an editor has
/// converted to CRLF throughout (judged by its first line) gets one `\r`
/// taken back off each line.
pub(crate) fn parse_notetype_file(text: &str) -> Result<ParsedNotetypeFile> {
    let crlf = text.split('\n').next().is_some_and(|l| l.ends_with('\r'));
    // Like `str::lines`, the newline ending the last line doesn't start
    // another (empty) one.
    let mut lines = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .map(|l| {
            if crlf {
                l.strip_suffix('\r').unwrap_or(l)
            } else {
                l
            }
        })
        .peekable();
    let mut parsed = ParsedNotetypeFile::default();

    while let Some(line) = lines.peek() {
        if line.starts_with("## ") {
            break;
        }
        if let Some(rest) = line.strip_prefix("# notetype:") {
            parsed.notetype = Some(rest.trim().to_string());
        }
        lines.next();
    }

    let mut sections: Vec<(String, String)> = vec![];
    for line in lines {
        if let Some(header) = line.strip_prefix("## ") {
            sections.push((header.trim_end().to_string(), String::new()));
        } else if let Some((_, content)) = sections.last_mut() {
            content.push_str(unescape_field_line(line));
            content.push('\n');
        } else {
            bail!("expected a `## ` section heading, found: {line}");
        }
    }

    let (mut fields, mut templates, mut css) = (None, None, None);
    for (header, content) in sections {
        let value = trim_field(content);
        let slot = match header.as_str() {
            "fields" => &mut fields,
            "templates" => &mut templates,
            "css" => &mut css,
            _ => {
                let (name, front) = parse_template_header(&header)?;
                let idx = match parsed.sides.iter().position(|(n, _, _)| *n == name) {
                    Some(idx) => idx,
                    None => {
                        parsed.sides.push((name.to_string(), None, None));
                        parsed.sides.len() - 1
                    }
                };
                let (_, f, b) = &mut parsed.sides[idx];
                let side = if front { f } else { b };
                if side.replace(value).is_some() {
                    bail!("duplicate `## {header}` section");
                }
                continue;
            }
        };
        if slot.replace(value).is_some() {
            bail!("duplicate `## {header}` section");
        }
    }

    let list = |value: Option<String>, header: &str| -> Result<Vec<String>> {
        let value = value.with_context(|| format!("missing `## {header}` section"))?;
        Ok(value.lines().map(str::to_string).collect())
    };
    parsed.fields = list(fields, "fields")?;
    parsed.templates = list(templates, "templates")?;
    parsed.css = css.context("missing `## css` section")?;
    Ok(parsed)
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
    if let Some(expected) = &parsed.notetype
        && expected != &notetype.name
    {
        bail!("file is for notetype `{expected}`, not `{}`", notetype.name);
    }
    let fields: Vec<&str> = notetype.fields.iter().map(|f| f.name.as_str()).collect();
    if parsed.fields != fields {
        bail!(
            "fields {:?} don't match notetype `{}` fields {fields:?}; changing fields \
             forces a full sync, so do it in Anki desktop",
            parsed.fields,
            notetype.name
        );
    }
    let templates: Vec<&str> = notetype.templates.iter().map(|t| t.name.as_str()).collect();
    if parsed.templates != templates {
        bail!(
            "templates {:?} don't match notetype `{}` templates {templates:?}; changing \
             templates forces a full sync, so do it in Anki desktop",
            parsed.templates,
            notetype.name
        );
    }
    if let Some((name, _, _)) = parsed
        .sides
        .iter()
        .find(|(name, _, _)| !templates.contains(&name.as_str()))
    {
        bail!("`## template: {name}` isn't in the `## templates` list");
    }

    for template in &mut notetype.templates {
        let side = |front: bool| {
            parsed
                .sides
                .iter()
                .find(|(name, _, _)| *name == template.name)
                .and_then(|(_, f, b)| if front { f.clone() } else { b.clone() })
                .with_context(|| {
                    let which = if front { "front" } else { "back" };
                    format!("missing `## template: {} ({which})` section", template.name)
                })
        };
        let (front, back) = (side(true)?, side(false)?);
        template.config.q_format = front;
        template.config.a_format = back;
    }
    notetype.config.css = parsed.css.clone();
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
    let original = col
        .get_notetype_by_name(name)?
        .with_context(|| format!("no such notetype: {name} (see `notetypes list`)"))?;
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
    // rslib would otherwise silently append `+` until the name is unique.
    if col.get_notetype_by_name(name)?.is_some() {
        bail!("notetype `{name}` already exists");
    }
    let source = col
        .get_notetype_by_name(from)?
        .with_context(|| format!("no such notetype: {from} (see `notetypes list`)"))?;
    let mut notetype = (*source).clone();
    if let Some(parsed) = &parsed {
        apply_notetype_file(&mut notetype, parsed)?;
    }
    notetype.id = NotetypeId(0);
    notetype.name = name.to_string();
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

    /// Sets awkward template/CSS content directly through rslib (lines that
    /// look like section headers or escapes, a `---`, a `\r`, trailing
    /// newlines), then checks `show --full` → `edit` is a no-op.
    #[test]
    fn full_show_round_trips_exactly_through_edit() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let mut nt = (*col
            .get_notetype_by_name("Basic (and reversed card)")
            .unwrap()
            .unwrap())
        .clone();
        nt.templates[0].config.q_format =
            "{{Front}}\n## not a header\n\\backslash\n---\n<br>\r\n\n".into();
        nt.templates[1].config.a_format = "{{FrontSide}}\n\n## also not\n\n\n".into();
        nt.config.css = ".card {}\n## css\n".into();
        col.update_notetype(&mut nt, false).unwrap();
        let stored = col.get_notetype_by_name(&nt.name).unwrap().unwrap();

        let shown = show_notetype(&mut col, &nt.name, true).unwrap();
        assert!(shown.contains("\n\\## not a header\n\\\\backslash\n\\---\n<br>\r\n"));
        let mut reparsed = (*stored).clone();
        apply_notetype_file(&mut reparsed, &parse_notetype_file(&shown).unwrap()).unwrap();
        assert_eq!(reparsed, *stored);

        assert_eq!(
            edit_notetype(&mut col, &nt.name, &shown).unwrap(),
            EditOutcome::Unchanged
        );
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
