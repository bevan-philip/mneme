mod ankiweb;
mod backup;
mod cards;
mod cli;
mod decks;
mod notetypes;
mod paths;
mod prefs;
mod profile;
mod store;
mod sync;

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
        Command::Sync => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            // Read the login before touching the collection, so a profile
            // that was never logged in fails without a pointless backup.
            let prefs = prefs::read_sync_prefs(&anki2_root, &profile_name)?;
            let auth = prefs::sync_auth(&prefs)?;
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            backup::backup_collection(&mut col, &collection_path)?;
            if let Some(user) = &prefs.sync_user {
                println!("syncing as {user}");
            }
            let output = ankiweb::normal_sync(&mut col, auth)?;
            print!("{}", ankiweb::describe_sync(&output)?);
        }
        Command::Decks { command } => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            match command {
                DecksCommand::List => {
                    print!("{}", decks::list_decks(&mut col)?);
                }
                DecksCommand::Add { name } => {
                    let deck_id = decks::add_deck(&mut col, &name)?;
                    println!("created deck {name} ({deck_id})");
                }
            }
        }
        Command::Notetypes { command } => {
            let profile_name = profile::resolve_profile(&anki2_root, requested_profile)?;
            let collection_path = paths::collection_path(&anki2_root, &profile_name);
            let mut col = store::open_collection(&collection_path).map_err(open_error_to_anyhow)?;
            match command {
                NotetypesCommand::List => {
                    print!("{}", notetypes::list_notetypes(&mut col)?);
                }
                NotetypesCommand::Show { name, full } => {
                    print!("{}", notetypes::show_notetype(&mut col, &name, full)?);
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
            if let Some(path) = input {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let parsed_notes = cards::parse_note_files_for_add(&text)?;
                let outcomes = cards::add_many(col, &notetype, &deck, &parsed_notes)?;

                let mut failed = 0;
                for (idx, outcome) in outcomes.iter().enumerate() {
                    match outcome {
                        cards::NoteOutcome::Added {
                            note_id,
                            card_count,
                        } => {
                            println!(
                                "note {}: added as {note_id} ({card_count} card(s))",
                                idx + 1
                            );
                        }
                        cards::NoteOutcome::Failed { error } => {
                            failed += 1;
                            println!("note {}: failed: {error}", idx + 1);
                        }
                    }
                }
                if failed > 0 {
                    anyhow::bail!("{failed} of {} note(s) failed to add", outcomes.len());
                }
            } else if !fields.is_empty() {
                let fields = cards::parse_field_args(&fields)?;
                let (note_id, card_count) = cards::add(col, &notetype, &deck, &fields, &[])?;
                println!("added note {note_id} ({card_count} card(s))");
            } else {
                anyhow::bail!("provide either --field NAME=VALUE or --input FILE");
            }
        }
        CardsCommand::Sync {
            input,
            create_deck,
            no_write_ids,
        } => {
            let text = std::fs::read_to_string(&input)
                .with_context(|| format!("failed to read {}", input.display()))?;
            let notes = sync::parse_sync_file(&text)?;
            let outcomes = sync::sync(col, &notes, create_deck)?;

            let (mut added, mut updated, mut unchanged, mut failed) = (0, 0, 0, 0);
            for (idx, outcome) in outcomes.iter().enumerate() {
                match outcome {
                    sync::SyncOutcome::Added {
                        note_id,
                        card_count,
                    } => {
                        added += 1;
                        println!(
                            "note {}: added as {note_id} ({card_count} card(s))",
                            idx + 1
                        );
                    }
                    sync::SyncOutcome::Updated { note_id } => {
                        updated += 1;
                        println!("note {}: updated {note_id}", idx + 1);
                    }
                    sync::SyncOutcome::Unchanged { .. } => unchanged += 1,
                    sync::SyncOutcome::Failed { error } => {
                        failed += 1;
                        println!("note {}: failed: {error}", idx + 1);
                    }
                }
            }
            println!("{added} added, {updated} updated, {unchanged} unchanged, {failed} failed");

            if added > 0 && !no_write_ids {
                let written = sync::write_note_ids(&text, &notes, &outcomes);
                std::fs::write(&input, written)
                    .with_context(|| format!("failed to write ids back to {}", input.display()))?;
                println!("wrote {added} new note id(s) to {}", input.display());
            }
            if failed > 0 {
                anyhow::bail!("{failed} of {} note(s) failed to sync", outcomes.len());
            }
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
