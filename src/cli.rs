use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;

#[derive(Parser)]
#[command(
    name = "anki-cli",
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
    /// Inspect notetypes (field names and order) in the collection
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
        /// Multiple notes are separated by a line containing exactly `---`;
        /// every note is validated up front (unknown field names, a
        /// mismatched `# notetype:` header) before any are added, so an
        /// invalid batch adds none of it. Each note is then added and
        /// reported on independently.
        #[arg(long)]
        input: Option<PathBuf>,
    },
}
