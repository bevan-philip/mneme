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
    /// there are several, one must be named explicitly.
    #[arg(long, global = true)]
    pub profile: Option<String>,

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
}
