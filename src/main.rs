mod ankiweb;
mod backup;
mod card_state;
mod cards;
mod cli;
mod decks;
mod notetypes;
mod paths;
mod prefs;
mod profile;
mod store;
mod sync;

use std::io::Read as _;

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
                NotetypesCommand::Show { name, full, output } => {
                    emit(&notetypes::show_notetype(&mut col, &name, full)?, output)?;
                }
                NotetypesCommand::Edit { name, input } => {
                    let text = read_text(&input)?;
                    match notetypes::edit_notetype(&mut col, &name, &text)? {
                        notetypes::EditOutcome::Updated => println!("updated notetype {name}"),
                        notetypes::EditOutcome::Unchanged => {
                            println!("notetype {name} unchanged")
                        }
                    }
                }
                NotetypesCommand::Add { name, from, input } => {
                    let text = input.as_deref().map(read_text).transpose()?;
                    let (id, name) =
                        notetypes::add_notetype(&mut col, &name, &from, text.as_deref())?;
                    println!("created notetype {name} ({id})");
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

/// Read a text file the way an editor would show it: a BOM is honoured
/// (UTF-8, or UTF-16 as Windows PowerShell 5.1 writes) and stripped, and
/// CRLF becomes LF, so nothing past here deals with either.
fn read_text(path: &std::path::Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    decode_text(&bytes).with_context(|| format!("failed to read {}", path.display()))
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    let mut text = String::new();
    // `utf8_passthru` keeps invalid UTF-8 an error instead of U+FFFD.
    encoding_rs_io::DecodeReaderBytesBuilder::new()
        .utf8_passthru(true)
        .strip_bom(true)
        .build(bytes)
        .read_to_string(&mut text)?;
    Ok(text.replace("\r\n", "\n"))
}

/// Print `text`, or write it to `output` byte for byte (shell redirection
/// can re-encode it).
fn emit(text: &str, output: Option<std::path::PathBuf>) -> Result<()> {
    match output {
        Some(path) => {
            std::fs::write(&path, text)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("wrote {}", path.display());
        }
        None => print!("{text}"),
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
            emit(&cards::show(col, card_id)?, output)?;
        }
        CardsCommand::Edit { card_id, input } => {
            let text = read_text(&input)?;
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
                let text = read_text(&path)?;
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
            let text = read_text(&input)?;
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
        CardsCommand::Suspend(cards) => {
            change_card_state(col, &cards, card_state::StateChange::Suspend)?
        }
        CardsCommand::Unsuspend(cards) => {
            change_card_state(col, &cards, card_state::StateChange::Unsuspend)?
        }
        CardsCommand::Bury(cards) => change_card_state(col, &cards, card_state::StateChange::Bury)?,
        CardsCommand::Unbury(cards) => {
            change_card_state(col, &cards, card_state::StateChange::Unbury)?
        }
        CardsCommand::Flag { flag, cards } => {
            let flag = card_state::parse_flag(&flag)?;
            change_card_state(col, &cards, card_state::StateChange::Flag(flag))?;
        }
    }
    Ok(())
}

fn change_card_state(
    col: &mut anki::collection::Collection,
    cards: &cli::CardSelection,
    change: card_state::StateChange,
) -> Result<()> {
    let card_ids = card_state::resolve_cards(col, &cards.card_ids, cards.search.as_deref())?;
    let changed = card_state::apply(col, &card_ids, change)?;
    println!("{}", card_state::describe(change, changed, card_ids.len()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_boms_and_crlf_to_plain_lf_text() {
        let utf16le: Vec<u8> = "\u{FEFF}a\r\nb\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        for bytes in [&b"a\r\nb\n"[..], b"\xEF\xBB\xBFa\nb\n", &utf16le] {
            assert_eq!(decode_text(bytes).unwrap(), "a\nb\n");
        }
        assert!(decode_text(b"\xEF\xBB\xBFa\xFF").is_err());
    }
}
