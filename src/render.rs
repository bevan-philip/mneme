use std::path::Path;
use std::sync::LazyLock;

use anki::card::Card;
use anki::card::CardId;
use anki::cloze::cloze_number_in_fields;
use anki::collection::Collection;
use anki::config::BoolKey;
use anki::decks::DeckId;
use anki::notes::Note;
use anki::notes::NoteId;
use anki::notetype::Notetype;
use anki::notetype::NotetypeKind;
use anki::services::CardRenderingService as _;
use anki_proto::card_rendering::ExtractAvTagsRequest;
use anki_proto::card_rendering::ExtractLatexRequest;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use regex::Regex;

use crate::cards::ParsedNoteFile;
use crate::cards::note_block_spans;
use crate::cards::parse_note_file;
use crate::cards::set_fields;

use crate::vendor;

/// One card, rendered as the two pages Anki's previewer shows for it.
pub struct RenderedCard {
    /// File-name stem: the card id for a stored card, `note<N>-card<M>` for
    /// one a note file would generate.
    pub name: String,
    pub front: String,
    pub back: String,
    /// Anything the preview can't show the way Anki would.
    pub warnings: Vec<String>,
    /// Whether either page loads MathJax from `PageOptions::mathjax_dir`.
    pub needs_mathjax: bool,
}

/// How the pages are put together, and what they link to outside
/// themselves (both absolute paths).
pub struct PageOptions<'a> {
    /// The collection's media folder: the card's media resolve against it.
    pub media_dir: &'a Path,
    /// Where `vendor::write_mathjax` puts MathJax for these pages.
    pub mathjax_dir: &'a Path,
    /// Show the cards as desktop does with its dark theme.
    pub night_mode: bool,
}

/// Render stored cards, through `templates` (a notetype with edited
/// templates/CSS, from `notetypes::preview_notetype`) instead of the stored
/// notetype when given.
pub fn render_cards(
    col: &mut Collection,
    card_ids: &[i64],
    templates: Option<&Notetype>,
    options: &PageOptions,
) -> Result<Vec<RenderedCard>> {
    let mut rendered = Vec::with_capacity(card_ids.len());
    for &cid in card_ids {
        let card = col
            .storage
            .get_card(CardId(cid))?
            .with_context(|| format!("no such card: {cid}"))?;
        let note = col
            .storage
            .get_note(card.note_id())?
            .context("card references a missing note")?;
        let notetype = col
            .get_notetype(note.notetype_id)?
            .context("note references a missing notetype")?;
        let notetype = with_templates(&notetype, templates, &format!("card {cid}"))?;
        rendered.push(render_one(
            col,
            cid.to_string(),
            &note,
            &card,
            notetype,
            options,
        )?);
    }
    Ok(rendered)
}

/// Render every card the notes in a note file would have, without adding
/// or changing anything. Headers resolve as in `cards sync`: a block's own
/// `# notetype:`/`# deck:`/`# tags:` win, anything omitted comes from the
/// first block. A block with a `# nid:` renders as that note would with the
/// block applied, through its existing cards where it has them.
///
/// The cards rendered are the ones desktop's card layout screen offers for
/// a note: every template of a normal notetype (an empty front renders
/// rslib's empty-card message, and is warned about), and for a cloze
/// notetype each cloze number in the note, or cloze 1 if there are none.
pub fn render_notes(
    col: &mut Collection,
    text: &str,
    templates: Option<&Notetype>,
    options: &PageOptions,
) -> Result<Vec<RenderedCard>> {
    let mut first: Option<ParsedNoteFile> = None;
    let mut rendered = vec![];
    let blocks = note_block_spans(text)
        .into_iter()
        .map(|span| &text[span])
        .filter(|block| !block.trim().is_empty());
    for (idx, block) in blocks.enumerate() {
        let n = idx + 1;
        let mut parsed = parse_note_file(block).with_context(|| format!("note {n}"))?;
        if let Some(first) = &first {
            parsed.notetype = parsed.notetype.or_else(|| first.notetype.clone());
            parsed.deck = parsed.deck.or_else(|| first.deck.clone());
            parsed.tags = parsed.tags.or_else(|| first.tags.clone());
        }
        let notetype_name = parsed
            .notetype
            .as_deref()
            .with_context(|| format!("note {n}: no `# notetype:` header"))?;
        let notetype = col.get_notetype_by_name(notetype_name)?.with_context(|| {
            format!("note {n}: no such notetype: {notetype_name} (see `notetypes list`)")
        })?;
        let notetype = with_templates(&notetype, templates, &format!("note {n}"))?;

        let (mut note, existing_cards) = match parsed.note_id {
            Some(id) => {
                let note = col
                    .storage
                    .get_note(NoteId(id))?
                    .with_context(|| format!("note {n}: no such note: {id}"))?;
                if note.notetype_id != notetype.id {
                    bail!("note {n}: note {id} is not a `{}` note", notetype.name);
                }
                (note, col.storage.all_cards_of_note(NoteId(id))?)
            }
            None => (Note::new(notetype), vec![]),
        };
        set_fields(&mut note, notetype, &parsed.fields).with_context(|| format!("note {n}"))?;
        if let Some(tags) = &parsed.tags {
            note.tags = tags.clone();
        }
        // Desktop previews a card that doesn't exist yet in the default deck.
        let deck_id = match &parsed.deck {
            Some(deck) => col.get_deck_id(deck)?,
            None => None,
        }
        .unwrap_or(DeckId(1));

        for ord in card_ords(&note, notetype) {
            let card = existing_cards
                .iter()
                .find(|card| card.template_idx() == ord)
                .cloned()
                .unwrap_or_else(|| Card::new(note.id, ord, deck_id, 0));
            let name = format!("note{n}-card{}", ord + 1);
            rendered.push(render_one(col, name, &note, &card, notetype, options)?);
        }
        first.get_or_insert(parsed);
    }
    if rendered.is_empty() {
        bail!("no `## FieldName` sections found");
    }
    Ok(rendered)
}

/// `templates` (the `--templates` preview of a notetype) if given, else the
/// stored notetype. A card of another notetype is an error rather than
/// silently rendered without the edit being previewed.
fn with_templates<'a>(
    stored: &'a Notetype,
    templates: Option<&'a Notetype>,
    what: &str,
) -> Result<&'a Notetype> {
    match templates {
        None => Ok(stored),
        Some(edited) if edited.id == stored.id => Ok(edited),
        Some(edited) => bail!(
            "{what} uses notetype `{}`, but --templates is for `{}`",
            stored.name,
            edited.name
        ),
    }
}

/// The card ordinals desktop's card layout screen offers for `note`.
fn card_ords(note: &Note, notetype: &Notetype) -> Vec<u16> {
    match notetype.config.kind() {
        NotetypeKind::Normal => (0..notetype.templates.len() as u16).collect(),
        NotetypeKind::Cloze => {
            let mut ords: Vec<u16> = cloze_number_in_fields(note.fields())
                .into_iter()
                // rslib's card generation caps cloze cards at 500.
                .map(|number| number.saturating_sub(1).min(499))
                .collect();
            ords.sort_unstable();
            ords.dedup();
            if ords.is_empty() {
                ords.push(0);
            }
            ords
        }
    }
}

/// Render one card the way desktop's browser previewer does
/// (`aqt/browser/previewer.py`): rslib renders the templates, the sound and
/// TTS tags are swapped for play references (`TemplateRenderContext.render`)
/// and LaTeX for its image (the `card_did_render` hook in `anki/latex.py`),
/// then `[[type:]]` boxes are dropped and play buttons added
/// (`prepare_card_text_for_display`).
fn render_one(
    col: &mut Collection,
    name: String,
    note: &Note,
    card: &Card,
    notetype: &Notetype,
    options: &PageOptions,
) -> Result<RenderedCard> {
    let template = match notetype.config.kind() {
        NotetypeKind::Normal => notetype.templates.get(card.template_idx() as usize),
        NotetypeKind::Cloze => notetype.templates.first(),
    }
    .with_context(|| format!("card {name}: its notetype has no matching template"))?;
    let output = col.render_card(note, card, notetype, template, false, false)?;
    let hide_play_buttons = col.get_config_bool(BoolKey::HideAudioPlayButtons);

    let mut warnings = vec![];
    if output.is_empty {
        warnings.push(
            "the front is empty, so Anki shows an empty-card message instead \
             (and doesn't create this card for a new note)"
                .to_string(),
        );
    }
    let mut side = |text: String, question_side: bool| -> Result<(String, bool)> {
        let text = col
            .extract_av_tags(ExtractAvTagsRequest {
                text,
                question_side,
            })?
            .text;
        let latex = col.extract_latex(ExtractLatexRequest {
            text,
            svg: notetype.config.latex_svg,
            expand_clozes: false,
        })?;
        for image in latex.latex {
            let warning = format!(
                "LaTeX image {} hasn't been generated yet; Anki desktop \
                 generates it the first time it shows the card",
                image.filename
            );
            // The back usually repeats the front's LaTeX via {{FrontSide}}.
            if !options.media_dir.join(&image.filename).exists() && !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        let text = col.encode_iri_paths(latex.text.into())?.val;
        Ok(preview_page(
            &text,
            &notetype.config.css,
            card.template_idx(),
            options,
            hide_play_buttons,
        ))
    };
    let (front, front_math) = side(output.question().into_owned(), true)?;
    let (back, back_math) = side(output.answer().into_owned(), false)?;
    Ok(RenderedCard {
        name,
        front,
        back,
        warnings,
        needs_mathjax: front_math || back_math,
    })
}

/// The reviewer page desktop's previewer puts a card side in, holding
/// `text` (fully rendered card HTML, play references not yet replaced),
/// and whether it loads MathJax.
fn preview_page(
    text: &str,
    css: &str,
    card_ord: u16,
    options: &PageOptions,
    hide_play_buttons: bool,
) -> (String, bool) {
    let text = TYPE_ANSWER.replace_all(text, "");
    let text = if hide_play_buttons {
        AV_REF.replace_all(&text, "")
    } else {
        AV_REF.replace_all(&text, PLAY_BUTTON)
    };
    let needs_mathjax = MATHJAX.is_match(&text);
    let mathjax = if needs_mathjax {
        let script = url::Url::from_file_path(options.mathjax_dir.join(vendor::MATHJAX_SCRIPT))
            .map(String::from)
            .unwrap_or_default();
        format!(
            "<script>\n{MATHJAX_CONFIG}</script>\n<script src=\"{}\"></script>\n",
            escape_attribute(&script)
        )
    } else {
        String::new()
    };
    let base = media_base_url(options.media_dir);
    let platform = if cfg!(windows) {
        "isWin"
    } else if cfg!(target_os = "macos") {
        "isMac"
    } else {
        "isLin"
    };
    // `theme_manager.body_classes_for_card_ord`, and the webview's
    // `night-mode` class on the root, which switches Anki's CSS variables.
    let (html_class, night_mode) = if options.night_mode {
        ("night-mode", " nightMode night_mode")
    } else {
        ("", "")
    };
    let class = format!("card card{} {platform}{night_mode} fancy", card_ord + 1);
    let page = format!(
        "<!doctype html>\n\
         <html class=\"{html_class}\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <base href=\"{}\">\n\
         <style>\n{}</style>\n\
         <style>\n{REVIEWER_CSS}</style>\n\
         {mathjax}\
         </head>\n\
         <body class=\"{class}\">\n\
         <div id=\"qa\" dir=\"auto\"><style>{css}</style>{text}</div>\n\
         </body>\n\
         </html>\n",
        escape_attribute(&base),
        vendor::ANKI_ROOT_VARS_CSS,
    );
    (page, needs_mathjax)
}

/// `media_dir` as a `file:` URL ending in `/`, so the card's relative media
/// references resolve against it, as they do against desktop's media server.
fn media_base_url(media_dir: &Path) -> String {
    url::Url::from_directory_path(media_dir)
        .map(String::from)
        .unwrap_or_default()
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// The previewer's `type_ans_preview_filter`: no answer box in a preview.
static TYPE_ANSWER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[type:[^]]+\]\]").unwrap());

/// `anki.sound.AV_REF_RE`.
static AV_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[anki:(play:(.):(\d+))\]").unwrap());

/// The reviewer's `mathjaxRegex`, which decides whether to load MathJax.
static MATHJAX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\\\[(.*?)\\\]|\\\((.*?)\\\)").unwrap());

/// `aqt.sound.av_refs_to_play_icons`, without the `pycmd` click handler
/// that only exists inside desktop.
const PLAY_BUTTON: &str = r#"
<a class="replay-button soundLink" href=# onclick="return false;" draggable="false">
    <svg class="playImage" viewBox="0 0 64 64" version="1.1">
        <circle cx="32" cy="32" r="29" />
        <path d="M56.502,32.301l-37.502,20.101l0.329,-40.804l37.173,20.703Z" />
    </svg>
</a>"#;

/// Desktop's MathJax setup (`ts/mathjax/index.ts`), minus its loader
/// path: MathJax finds its fonts relative to its own script.
const MATHJAX_CONFIG: &str = r#"window.MathJax = {
    tex: {
        displayMath: [["\\[", "\\]"]],
        processEscapes: false,
        processEnvironments: false,
        processRefs: false,
        packages: { "[+]": ["noerrors", "mathtools"], "[-]": ["textmacros"] },
    },
    loader: { load: ["[tex]/noerrors", "[tex]/mathtools"] },
};
"#;

/// The parts of desktop's reviewer stylesheet (`ts/reviewer/reviewer.scss`)
/// that apply to card content. `body.nightMode` is what overrides a stock
/// `.card`'s black-on-white in night mode.
const REVIEWER_CSS: &str = "\
hr { background-color: #737373; margin: 1em 0; border: none; height: 1px; }
body { margin: 20px; overflow-wrap: break-word; }
body.nightMode { background-color: var(--canvas); color: var(--fg); }
img { max-width: 100%; max-height: 95vh; }
li { text-align: start; }
pre { text-align: left; }
.replay-button { text-decoration: none; display: inline-flex; vertical-align: middle; margin: 3px; }
.replay-button svg { width: 40px; height: 40px; }
.replay-button svg circle { fill: #fff; stroke: #414141; }
.replay-button svg path { fill: #414141; }
.drawing { zoom: 50%; }
.nightMode .latex { filter: invert(100%); }
.nightMode img.drawing { filter: invert(1) hue-rotate(180deg); }
";

#[cfg(test)]
mod tests {
    use anki::collection::CollectionBuilder;
    use anki::search::SortMode;

    use super::*;

    fn options() -> PageOptions<'static> {
        PageOptions {
            media_dir: Path::new("/m"),
            mathjax_dir: Path::new("/out/mathjax"),
            night_mode: false,
        }
    }

    fn add_basic(col: &mut Collection, notetype: &str, front: &str, back: &str) -> Vec<i64> {
        col.get_or_create_normal_deck("Default").unwrap();
        let fields = vec![
            ("Front".to_string(), front.to_string()),
            ("Back".to_string(), back.to_string()),
        ];
        let (nid, _) = crate::cards::add(col, notetype, "Default", &fields, &[]).unwrap();
        col.search_cards(format!("nid:{nid}").as_str(), SortMode::NoOrder)
            .unwrap()
            .into_iter()
            .map(|c| c.0)
            .collect()
    }

    fn qa(page: &str) -> &str {
        let start = page.find("<div id=\"qa\" dir=\"auto\">").unwrap();
        let end = page.rfind("</div>").unwrap();
        &page[start..end]
    }

    #[test]
    fn renders_both_sides_with_the_notetype_css_and_card_class() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(
            &mut col,
            "Basic (and reversed card)",
            "hola",
            "<b>hello</b>",
        );
        let options = PageOptions {
            media_dir: Path::new("/media dir"),
            ..options()
        };

        let rendered = render_cards(&mut col, &cards, None, &options).unwrap();

        assert_eq!(rendered.len(), 2);
        let first = &rendered[0];
        assert_eq!(first.name, cards[0].to_string());
        assert!(first.front.contains("<body class=\"card card1 "));
        assert!(qa(&first.front).ends_with("</style>hola"));
        assert!(qa(&first.back).ends_with("</style>hola\n\n<hr id=answer>\n\n<b>hello</b>"));
        assert!(first.front.contains(".card {"));
        assert!(first.front.contains("<base href=\"file:///media%20dir/\">"));
        assert!(rendered[1].front.contains("<body class=\"card card2 "));
        assert!(qa(&rendered[1].front).ends_with("</style><b>hello</b>"));
        assert!(first.warnings.is_empty());
    }

    #[test]
    fn renders_every_template_of_a_new_note_without_adding_it() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let text = "# notetype: Basic (and reversed card)\n\n## Front\nhola\n\n## Back\nhello\n";

        let rendered = render_notes(&mut col, text, None, &options()).unwrap();

        let names: Vec<&str> = rendered.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["note1-card1", "note1-card2"]);
        assert!(qa(&rendered[0].front).ends_with("</style>hola"));
        assert!(qa(&rendered[1].front).ends_with("</style>hello"));
        assert!(rendered[1].front.contains("<body class=\"card card2 "));
        assert!(col.search_cards("", SortMode::NoOrder).unwrap().is_empty());
    }

    #[test]
    fn renders_one_card_per_cloze_number() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let text = "# notetype: Cloze\n\n## Text\n{{c1::a}} {{c3::b}}\n\n## Back Extra\n\n";

        let rendered = render_notes(&mut col, text, None, &options()).unwrap();

        let names: Vec<&str> = rendered.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["note1-card1", "note1-card3"]);
        assert!(qa(&rendered[1].front).ends_with(
            "<span class=\"cloze-inactive\" data-ordinal=\"1\">a</span> \
                 <span class=\"cloze\" data-cloze=\"b\" data-ordinal=\"3\">[...]</span>"
        ));
        assert!(rendered[1].front.contains("<body class=\"card card3 "));
    }

    #[test]
    fn later_blocks_inherit_the_first_blocks_notetype() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let text = "# notetype: Basic\n\n## Front\none\n\n## Back\na\n\n---\n## Front\ntwo\n\n## Back\nb\n";

        let rendered = render_notes(&mut col, text, None, &options()).unwrap();

        let names: Vec<&str> = rendered.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["note1-card1", "note2-card1"]);
        assert!(qa(&rendered[1].front).ends_with("</style>two"));
    }

    #[test]
    fn a_block_with_a_nid_renders_the_edit_through_the_existing_card() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic", "old", "a");
        let shown = crate::cards::show(&mut col, cards[0]).unwrap();
        let nid = col
            .storage
            .get_card(CardId(cards[0]))
            .unwrap()
            .unwrap()
            .note_id();
        let text = format!("# nid: {}\n{}", nid.0, shown.replace("old", "new"));
        let mut tmpl = (*col.get_notetype_by_name("Basic").unwrap().unwrap()).clone();
        tmpl.templates[0].config.q_format = "{{Front}} {{CardID}}".into();
        col.update_notetype(&mut tmpl, false).unwrap();

        let rendered = render_notes(&mut col, &text, None, &options()).unwrap();

        assert!(qa(&rendered[0].front).ends_with(&format!("</style>new {}", cards[0])));
        let note = col.storage.get_note(nid).unwrap().unwrap();
        assert_eq!(note.fields()[0], "old");
    }

    #[test]
    fn rejects_fields_that_dont_match_the_notetype() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let text = "# notetype: Basic\n\n## Fornt\nhola\n\n## Back\nhello\n";
        let err = render_notes(&mut col, text, None, &options())
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("don't match notetype"));

        let err = render_notes(&mut col, "## Front\nq\n", None, &options())
            .err()
            .unwrap();
        assert!(err.to_string().contains("no `# notetype:` header"));
    }

    #[test]
    fn warns_about_a_card_with_an_empty_front() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let text = "# notetype: Basic (optional reversed card)\n\n## Front\nq\n\n## Back\na\n\n## Add Reverse\n\n";

        let rendered = render_notes(&mut col, text, None, &options()).unwrap();

        assert!(rendered[0].warnings.is_empty());
        assert_eq!(rendered[1].warnings.len(), 1);
        assert!(rendered[1].warnings[0].contains("front is empty"));
    }

    fn edited_basic(col: &mut Collection) -> Notetype {
        let shown = crate::notetypes::show_notetype(col, "Basic", true).unwrap();
        let edited = shown
            .replace(
                "## template: Card 1 (front)\n{{Front}}",
                "## template: Card 1 (front)\n<i>{{Front}}</i>",
            )
            .replace(".card {", ".card { border: 1px solid red;");
        crate::notetypes::preview_notetype(col, &edited).unwrap()
    }

    #[test]
    fn renders_through_edited_templates_without_saving_them() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic", "hola", "hello");
        let edited = edited_basic(&mut col);

        let rendered = &render_cards(&mut col, &cards, Some(&edited), &options()).unwrap()[0];

        assert!(qa(&rendered.front).ends_with("</style><i>hola</i>"));
        assert!(qa(&rendered.back).contains("</style><i>hola</i>\n\n<hr id=answer>"));
        assert!(rendered.front.contains(".card { border: 1px solid red;"));
        let stored = col.get_notetype_by_name("Basic").unwrap().unwrap();
        assert_eq!(stored.templates[0].config.q_format, "{{Front}}");

        let text = "# notetype: Basic\n\n## Front\nnew\n\n## Back\nb\n";
        let rendered = &render_notes(&mut col, text, Some(&edited), &options()).unwrap()[0];
        assert!(qa(&rendered.front).ends_with("</style><i>new</i>"));
    }

    #[test]
    fn rejects_templates_for_another_notetype() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic (and reversed card)", "hola", "hello");
        let edited = edited_basic(&mut col);

        let err = render_cards(&mut col, &cards, Some(&edited), &options())
            .err()
            .unwrap();
        assert!(err.to_string().contains("--templates is for `Basic`"));
    }

    #[test]
    fn night_mode_sets_desktops_classes_and_dark_variables() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic", "q", "a");

        let light = &render_cards(&mut col, &cards, None, &options()).unwrap()[0].front;
        assert!(light.contains("<html class=\"\">"));
        assert!(!light.contains("nightMode night_mode"));

        let options = PageOptions {
            night_mode: true,
            ..options()
        };
        let night = &render_cards(&mut col, &cards, None, &options).unwrap()[0].front;
        assert!(night.contains("<html class=\"night-mode\">"));
        assert!(night.contains(" nightMode night_mode fancy\">"));
        assert!(night.contains(":root.night-mode {"));
        assert!(night.contains("body.nightMode { background-color: var(--canvas);"));
    }

    #[test]
    fn errors_on_an_unknown_card() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let err = render_cards(&mut col, &[123], None, &options())
            .err()
            .unwrap();
        assert!(err.to_string().contains("no such card: 123"));
    }

    #[test]
    fn swaps_sounds_for_play_buttons_and_encodes_media_names() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(
            &mut col,
            "Basic",
            "[sound:a b.mp3]<img src=\"c d.png\">",
            "x",
        );

        let page = &render_cards(&mut col, &cards, None, &options()).unwrap()[0].front;

        assert!(!page.contains("[sound:"));
        assert!(!page.contains("[anki:play"));
        assert!(qa(page).contains("<a class=\"replay-button soundLink\""));
        assert!(page.contains("<img src=\"c%20d.png\">"));
    }

    #[test]
    fn hides_play_buttons_when_desktop_is_set_to() {
        let mut col = CollectionBuilder::default().build().unwrap();
        col.set_config_bool(BoolKey::HideAudioPlayButtons, true, false)
            .unwrap();
        let cards = add_basic(&mut col, "Basic", "[sound:a.mp3]q", "x");

        let page = &render_cards(&mut col, &cards, None, &options()).unwrap()[0].front;

        assert!(qa(page).ends_with("</style>q"));
    }

    #[test]
    fn drops_type_answer_boxes_like_the_previewer() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic (type in the answer)", "q", "a");

        let rendered = &render_cards(&mut col, &cards, None, &options()).unwrap()[0];

        assert!(!rendered.front.contains("[[type:"));
        assert!(!rendered.back.contains("[[type:"));
    }

    #[test]
    fn loads_mathjax_only_for_math() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic", r"\(x^2\)", "plain");

        let rendered = &render_cards(&mut col, &cards, None, &options()).unwrap()[0];
        assert!(rendered.needs_mathjax);
        assert!(
            rendered
                .front
                .contains("<script src=\"file:///out/mathjax/tex-chtml-full.js\"></script>")
        );

        let cards = add_basic(&mut col, "Basic", "plain", "plain");
        let rendered = &render_cards(&mut col, &cards, None, &options()).unwrap()[0];
        assert!(!rendered.needs_mathjax);
        assert!(!rendered.front.contains("<script"));
    }

    #[test]
    fn links_latex_images_and_warns_about_ungenerated_ones() {
        let mut col = CollectionBuilder::default().build().unwrap();
        let cards = add_basic(&mut col, "Basic", "[$]x^2[/$]", "a");
        let media = tempfile::tempdir().unwrap();
        let options = PageOptions {
            media_dir: media.path(),
            ..options()
        };

        let rendered = &render_cards(&mut col, &cards, None, &options).unwrap()[0];
        assert!(
            qa(&rendered.front)
                .contains("<img class=latex alt=\"&#x24;x&#x5E;2&#x24;\" src=\"latex-")
        );
        assert_eq!(rendered.warnings.len(), 1);
        let image = rendered.warnings[0].split(' ').nth(2).unwrap();
        assert!(image.starts_with("latex-"));

        std::fs::write(media.path().join(image), "").unwrap();
        let rendered = &render_cards(&mut col, &cards, None, &options).unwrap()[0];
        assert!(rendered.warnings.is_empty());
    }
}
