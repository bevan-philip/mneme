use std::fmt::Write as _;

use anki::collection::Collection;
use anki::notetype::Notetype;
use anyhow::Context;
use anyhow::Result;

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
/// note-file format expect, plus its template names.
pub fn show_notetype(col: &mut Collection, name: &str) -> Result<String> {
    let notetype = col
        .get_notetype_by_name(name)?
        .with_context(|| format!("no such notetype: {name} (see `notetypes list`)"))?;
    Ok(render_notetype(&notetype))
}

fn render_notetype(notetype: &Notetype) -> String {
    let mut out = String::new();
    writeln!(out, "# notetype: {}", notetype.name).unwrap();
    out.push('\n');
    writeln!(out, "## fields").unwrap();
    for field in &notetype.fields {
        writeln!(out, "{}", field.name).unwrap();
    }
    out.push('\n');
    writeln!(out, "## templates").unwrap();
    for template in &notetype.templates {
        writeln!(out, "{}", template.name).unwrap();
    }
    out
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

        let shown = show_notetype(&mut col, "Basic (and reversed card)").unwrap();

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
    fn errors_on_unknown_notetype() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let err = show_notetype(&mut col, "No Such Notetype").unwrap_err();
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
