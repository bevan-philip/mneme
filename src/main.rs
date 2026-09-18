mod backup;
mod cards;
mod cli;
mod decks;
mod notetypes;
mod paths;
mod profile;
mod store;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;

use cli::CardsCommand;
use cli::Cli;
use cli::Command;
use cli::DecksCommand;
use cli::NotetypesCommand;
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
    let requested_profile = cli.profile.as_deref().map(std::path::Path::as_os_str);

    match cli.command {
        Command::Profile {
            command: ProfileCommand::List,
        } => {
            let profiles = profile::discover_profiles(&anki2_root)?;
            if profiles.is_empty() {
                println!("no profiles found under {}", anki2_root.display());
            } else {
                for name in profiles {
                    println!("{}", name.to_string_lossy());
                }
            }
        }
        Command::Backup => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            let dest = backup::backup_collection(&mut col, &collection_path)?;
            println!(
                "backed up {} to {}",
                collection_path.display(),
                dest.display()
            );
        }
        Command::Decks {
            command: DecksCommand::List,
        } => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            print!("{}", decks::list_decks(&mut col)?);
        }
        Command::Notetypes { command } => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            match command {
                NotetypesCommand::List => {
                    print!("{}", notetypes::list_notetypes(&mut col)?);
                }
                NotetypesCommand::Show { name } => {
                    print!("{}", notetypes::show_notetype(&mut col, &name)?);
                }
            }
        }
        Command::Cards { command } => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            run_cards_command(&mut col, command)?;
        }
    }

    Ok(())
}

fn run_cards_command(col: &mut anki::collection::Collection, command: CardsCommand) -> Result<()> {
    match command {
        CardsCommand::List { search } => {
            let listing = cards::list_cards(col, search.as_deref().unwrap_or(""))?;
            print!("{listing}");
        }
        CardsCommand::Show { card_id, output } => {
            let text = cards::show(col, card_id)?;
            match output {
                Some(path) => {
                    std::fs::write(&path, &text)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    println!("wrote {}", path.display());
                }
                None => print!("{text}"),
            }
        }
        CardsCommand::Edit { card_id, input } => {
            let text = std::fs::read_to_string(&input)
                .with_context(|| format!("failed to read {}", input.display()))?;
            cards::edit(col, card_id, &text)?;
            println!("updated card {card_id}");
        }
        CardsCommand::Add {
            notetype,
            deck,
            fields,
            input,
        } => {
            let (fields, tags) = if let Some(path) = input {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let (file_notetype, tags, fields) = cards::parse_note_file_for_add(&text)?;
                if let Some(file_notetype) = file_notetype
                    && file_notetype != notetype
                {
                    anyhow::bail!(
                        "file is for notetype `{file_notetype}`, but --notetype was `{notetype}`"
                    );
                }
                (fields, tags)
            } else if !fields.is_empty() {
                (cards::parse_field_args(&fields)?, Vec::new())
            } else {
                anyhow::bail!("provide either --field NAME=VALUE or --input FILE");
            };
            let (note_id, card_count) = cards::add(col, &notetype, &deck, &fields, &tags)?;
            println!("added note {note_id} ({card_count} card(s))");
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
