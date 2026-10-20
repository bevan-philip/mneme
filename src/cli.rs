use std::path::PathBuf;

use clap::Args;
use clap::Parser;
use clap::Subcommand;

#[derive(Parser)]
#[command(
    name = "mneme-anki",
    about = "A CLI for directly editing your Anki collection"
)]
pub struct Cli {
    /// Anki profile to use. Defaults to the only profile found on disk; if
    /// there are several, one must be named explicitly. Typed as a raw path
    /// component (not `String`) so a non-UTF-8 profile directory name can
    /// still be targeted exactly.
    #[arg(long, global = true)]
    pub profile: Option<PathBuf>,

    /// Override the Anki2 data root (defaults to the platform's Anki2
    /// folder, e.g. ~/.local/share/Anki2 on Linux).
    #[arg(long, global = true)]
    pub anki2_root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Discover Anki profiles on this machine
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Inspect decks in the collection
    Decks {
        #[command(subcommand)]
        command: DecksCommand,
    },
    /// Inspect notetypes, and add or restyle them
    Notetypes {
        #[command(subcommand)]
        command: NotetypesCommand,
    },
    /// Read and write cards (and the notes behind them)
    Cards {
        #[command(subcommand)]
        command: CardsCommand,
    },
    /// Copy the collection file to a scratch backup location
    Backup,
    /// Two-way sync with AnkiWeb, using the login Anki desktop saved for
    /// this profile. Backs up first. Doesn't sync media, and stops without
    /// changing anything if AnkiWeb asks for a one-way full sync.
    Sync,
}

#[derive(Subcommand)]
pub enum ProfileCommand {
    /// List Anki profiles found on this machine
    List,
}

#[derive(Subcommand)]
pub enum DecksCommand {
    /// List all decks in the collection
    List,
    /// Create a new deck, including any missing `::`-nested parents
    Add {
        /// Deck name, e.g. "Spanish::Vocab::Week3"
        name: String,
    },
}

#[derive(Subcommand)]
pub enum NotetypesCommand {
    /// List all notetypes in the collection, by name and id
    List,
    /// Show a notetype's field names (in the order `cards add`/the
    /// note-file format expect) and template names
    Show {
        /// Notetype name, as printed by `notetypes list`
        name: String,
        /// Also print each template's front/back HTML and the CSS
        #[arg(long)]
        full: bool,
        /// Write to this file instead of stdout, byte for byte (shell
        /// redirection can re-encode it, e.g. as UTF-16 in Windows
        /// PowerShell 5.1)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Replace a notetype's card templates (front/back HTML) and CSS from a
    /// file in `show --full` format. Its field and template lists must match
    /// the notetype's exactly: changing either forces a one-way full sync,
    /// so that stays in Anki desktop
    Edit {
        /// Notetype name, as printed by `notetypes list`
        name: String,
        /// File to read, as written by `notetypes show --full`
        #[arg(long)]
        input: PathBuf,
    },
    /// Create a notetype by cloning an existing one, like Anki's Manage Note
    /// Types → Add → Clone, optionally with templates and CSS from a file
    Add {
        /// Name for the new notetype; must not already exist
        name: String,
        /// Existing notetype to clone
        #[arg(long)]
        from: String,
        /// Apply templates and CSS from this file (`show --full` format of
        /// the `--from` notetype, same rules as `edit`)
        #[arg(long)]
        input: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum CardsCommand {
    /// List cards, one per line, optionally filtered by an Anki search
    /// query (e.g. `deck:Spanish tag:verb`)
    List {
        /// Anki search query; matches every card when omitted
        search: Option<String>,
    },
    /// Dump a card's underlying note as an editable text file, since a
    /// card's content is really its note's fields (raw HTML) rendered
    /// through a template
    Show {
        /// Card id, as printed by `cards list`
        card_id: i64,
        /// Write to this file instead of stdout, handy for editing HTML
        /// fields in a text editor before feeding the file to `cards edit`
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Apply a note file (from `cards show`) back to a card
    Edit {
        /// Card id, as printed by `cards list`
        card_id: i64,
        /// File to read the edited note content from
        #[arg(long)]
        input: PathBuf,
    },
    /// Add a new note (and its generated cards)
    Add {
        /// Notetype to use, e.g. "Basic"
        #[arg(long)]
        notetype: String,
        /// Deck to place the new cards in
        #[arg(long)]
        deck: String,
        /// A field value as NAME=VALUE; repeat once per notetype field, in
        /// the notetype's own order. Mutually exclusive with --input
        #[arg(long = "field", value_name = "NAME=VALUE", conflicts_with = "input")]
        fields: Vec<String>,
        /// Read one or more notes (fields and tags) from a file in the same
        /// format as `cards show`, instead of passing --field repeatedly.
        /// Multiple notes are separated by a line containing exactly `---`,
        /// preceded by a blank line (or at the very start of the file);
        /// every note is validated up front (unknown field names, a
        /// mismatched `# notetype:` header) before any are added, so an
        /// invalid batch adds none of it. Each note is then added and
        /// reported on independently.
        #[arg(long)]
        input: Option<PathBuf>,
    },
    /// Add or update notes from a note file, making the file the source of
    /// truth. Each note block may carry `# notetype:`, `# deck:` and
    /// `# tags:` headers (later blocks inherit any the first block sets) and
    /// a `# nid:` header: blocks with a nid update that note, blocks without
    /// one are added and get their new nid written back into the file.
    /// Everything is validated before anything is written. Notes removed
    /// from the file are left alone in Anki.
    Sync {
        /// Note file to sync (same format as `cards add --input`)
        #[arg(long)]
        input: PathBuf,
        /// Create any deck named in the file that doesn't exist yet
        #[arg(long)]
        create_deck: bool,
        /// Don't write new note ids back into the file
        #[arg(long)]
        no_write_ids: bool,
    },
    /// Suspend cards, like Anki's browser
    Suspend(CardSelection),
    /// Unsuspend suspended cards (buried cards stay buried)
    Unsuspend(CardSelection),
    /// Bury cards until the next day, like Anki's manual bury (suspended
    /// cards are left suspended)
    Bury(CardSelection),
    /// Unbury buried cards (suspended cards stay suspended)
    Unbury(CardSelection),
    /// Set or clear cards' flag
    Flag {
        /// 0-7, or a flag name: none, red, orange, green, blue, pink,
        /// turquoise, purple (0/none clears the flag)
        flag: String,
        #[command(flatten)]
        cards: CardSelection,
    },
}

/// Which cards a state change applies to. To act on every card of a note,
/// use `--search nid:NOTE_ID`.
#[derive(Args)]
pub struct CardSelection {
    /// Card ids, as printed by `cards list`
    #[arg(required_unless_present = "search")]
    pub card_ids: Vec<i64>,
    /// Act on every card matching this Anki search query instead (e.g.
    /// `deck:Spanish tag:leech`)
    #[arg(long, conflicts_with = "card_ids")]
    pub search: Option<String>,
}
