mod backup;
mod cli;
mod decks;
mod paths;
mod profile;
mod store;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;

use cli::Cli;
use cli::Command;
use cli::DecksCommand;
use cli::ProfileCommand;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let anki2_root = cli
        .anki2_root
        .clone()
        .or_else(paths::default_anki2_root)
        .context(
            "could not determine the Anki2 data directory for this platform; pass --anki2-root",
        )?;

    match cli.command {
        Command::Profile {
            command: ProfileCommand::List,
        } => {
            let profiles = profile::discover_profiles(&anki2_root)?;
            if profiles.is_empty() {
                println!("no profiles found under {}", anki2_root.display());
            } else {
                for name in profiles {
                    println!("{name}");
                }
            }
        }
        Command::Backup => {
            let profile_name = profile::resolve_profile(&anki2_root, cli.profile.as_deref())?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            let dest = backup::backup_collection(&mut col, &anki2_root, &profile_name)?;
            println!(
                "backed up {} to {}",
                collection_path.display(),
                dest.display()
            );
        }
        Command::Decks {
            command: DecksCommand::List,
        } => {
            let profile_name = profile::resolve_profile(&anki2_root, cli.profile.as_deref())?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            print!("{}", decks::list_decks(&mut col)?);
        }
    }

    Ok(())
}

fn open_error_to_anyhow(err: store::OpenError) -> anyhow::Error {
    match err {
        store::OpenError::Locked => {
            anyhow::anyhow!("Anki's collection is locked; please close Anki Desktop and try again")
        }
        store::OpenError::Other(err) => {
            anyhow::Error::from(err).context("failed to open collection")
        }
    }
}
