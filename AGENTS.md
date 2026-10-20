- See @README.md for the core ideas of this application.
- Follow the imperative shell, functional core approach from Boundaries, by Gary Bernhardt.
- Leverage Anki's rslib religiously.
- Use the "LLM generated notes" section for any further notes you want to create.
- Do not modify README.md.
- Keep documentation to a minimum.

## LLM generated notes

- `anki` (rslib) and `anki_proto` are pulled straight from
  `github.com/ankitects/anki`, pinned by `rev` in `Cargo.toml` (no crates.io
  release exists). Bumping the pin means re-checking `src/store.rs` and
  `src/decks.rs` against upstream, since rslib has no semver guarantees.
- `rust-toolchain.toml` pins the compiler. When bumping rslib, keep it at or
  above the pinned anki rev's own `rust-toolchain.toml`, since rslib may use
  features newer than its declared `rust-version`.
- rslib's own `Cargo.toml` uses `tokio.workspace = true` etc., resolved
  against the anki repo's workspace-level feature set. That set only
  includes what the *whole* anki workspace needs, not what our smaller
  dependency graph needs — e.g. we had to add our own `tokio` dependency
  with the `io-util` feature so `AsyncReadExt`/`AsyncWriteExt` resolve
  inside rslib's sync code, even though we never call it. If a future
  rslib bump fails with unresolved-import errors from inside
  `rslib/src/...`, it's almost certainly the same issue: add/extend a
  direct dependency here with the missing feature.
- There's no way to compile rslib without its sync module (no feature
  flag upstream); we simply never call it.
- Layout follows functional core / imperative shell: `paths.rs` and the
  rendering half of `decks.rs` are pure; `profile.rs`, `backup.rs`,
  `store.rs`, and `main.rs` do the I/O and own the rslib `Collection`.
- Profile default-detection doesn't parse `prefs21.db` (it's a pickled
  Python value) — `resolve_profile` requires `--profile` whenever more than
  one profile directory exists.
- `backup_collection` goes through `Collection::maybe_backup`, not
  `fs::copy` of `collection.anki2`: a raw file copy can miss committed
  data still sitting in the WAL, and skips the lock check that opening a
  `Collection` gives us for free. rslib's own backup writes a `.colpkg`
  with second-granularity, self-thinning naming, which is also good
  enough collision resistance for us — no need to invent our own.
- Backups are written under a `<fingerprint(collection_path)>-<label>`
  subdirectory, fingerprinting the whole (lexically-absolute) collection
  path rather than root and profile separately: rslib's thinning treats
  every file in the directory it's given as one collection's retention
  history, so anything short of a per-collection directory risks one
  backup pruning another's recovery points. `<label>` is just the profile
  name for browsability and doesn't need to be collision-resistant itself.
  `paths::fingerprint` is a plain FNV-1a hash — no new dependency, no need
  for the path to exist yet — over the platform's lossless byte/UTF-16 view
  of the path (`OsStrExt`), not `to_string_lossy()`: on Unix, two different
  non-UTF-8 paths can both collapse to the same replacement-character
  string and so collide.
- Profile names are carried as `OsString`/`&OsStr` (`PathBuf` for the CLI
  flag) from `discover_profiles` through `resolve_profile` to
  `paths::collection_path`, not lossy-converted to `String` until the
  point of display. A profile directory could in principle contain
  non-UTF-8 bytes; `to_string_lossy()`-ing it before rejoining to a path
  would target a directory that doesn't exist, and `CollectionBuilder`
  would then silently create a fresh, wrong collection there instead of
  opening the real one.
- `resolve_profile` checks `collection.anki2` exists even for an explicit
  `--profile`, not just during auto-discovery: `CollectionBuilder` always
  creates a missing collection rather than erroring, so without this
  check a typo'd `--profile` would silently create and operate on a
  fresh, empty collection — exactly the "arbitrary database" behavior
  README says this CLI isn't for.
- `resolve_profile` also rejects an explicit `--profile` that isn't
  exactly one normal path component (`paths::is_profile_name`): it's a
  `PathBuf` on the `Cli` struct for lossless non-UTF-8 handling, but it's
  documented as a profile *name*, and joining an unvalidated one onto
  `anki2_root` would let `../elsewhere` or an absolute value silently
  target a collection outside the selected root.
- `backup_root`'s `<label>` is truncated (`paths::truncate_label`, 64
  bytes) before joining: it's cosmetic only (the fingerprint already
  carries the identity), but an untruncated, otherwise-valid long profile
  name could push the directory component past a filesystem's per-
  component limit (e.g. ext4's 255 bytes) and make `create_dir_all` fail
  for a profile that's perfectly fine on its own.
- `decks.rs` has a test (`indents_nested_decks_by_level`) pinning rslib's
  deck-tree level numbering against a real in-memory collection: the
  synthetic root is level 0, so real top-level decks are level 1 (no
  indent) and their children level 2. Worth keeping since a review bot
  once modeled this backwards — trust this test over a re-derivation from
  reading the rslib source.
- `cards.rs` edits notes, not cards: a card is a (note, template) pairing
  rslib generates and re-generates on its own (`Collection::update_note`
  already regenerates/removes cards for e.g. cloze edits), and its only
  independently editable content is the note's fields. `cards edit` takes
  a card id purely as the ergonomic handle a user has (from `cards list`
  or Anki's own browser) and resolves it to the underlying note; editing
  one card of a multi-template note (e.g. "Basic (and reversed card)")
  edits all of that note's cards, same as Anki's own editor.
- The `show`/`edit`/`add --input` file format (`render_note_file` /
  `parse_note_file` in `cards.rs`) is a plain-text, Markdown-flavoured
  serialization of a note chosen so field HTML round-trips through a text
  editor: a `# notetype:`/`# tags:` header for context, then one
  `## FieldName` section per field holding that field's raw content
  verbatim. Fields are matched by exact name and notetype order (not
  reordered by name) so a misspelled field name is a hard error instead of
  silently landing in the wrong slot.
- Two details make that round-trip exact rather than approximate, both
  covered by tests: (1) a field body line starting with `## ` (or `\`)
  would otherwise be misread as the next section boundary, so
  `escape_field`/`unescape_field_line` backslash-escape it on the way out
  and back; (2) the boundary `render_note_file` writes after each value is
  a *fixed* two newlines regardless of what the value itself ends with, so
  `trim_field` can strip exactly that much back off instead of
  `trim_end_matches('\n')`-ing away trailing newlines that were actually
  part of the field's own content.
- `cards add`/`cards edit` don't auto-create the deck or notetype named on
  the command line: `Note::new`/`add_note` would happily proceed with
  whatever's found, but a typo'd `--deck`/`--notetype` silently landing in
  the wrong place (or creating a stray deck) is worse than an error naming
  `decks list` as the way to check what exists.
- `cards list`'s tab-separated output sanitizes every column (deck name,
  notetype name, question preview) by replacing literal tabs with spaces —
  otherwise a field containing a tab produces a row with more than four
  columns, breaking anything parsing the listing.
- Input files are decoded once, in `main::read_text`: `encoding_rs_io`
  honours and strips a BOM (UTF-8, or UTF-16 as Windows PowerShell 5.1
  writes), and CRLF becomes LF, so no parser deals with either and
  `cards sync` writes a CRLF file back as LF. Parsers split with
  `str::lines()`. For note fields that loses nothing: rslib strips `\r`
  from fields anyway (pinned by
  `rslib_normalizes_carriage_returns_out_of_fields`).
- `cards add --input` accepts a file holding several notes, separated by a
  line containing exactly `---` (`NOTE_DELIMITER`/`split_note_blocks` in
  `cards.rs`); a file with no such line is still just one note, so this is
  fully backward compatible with `cards show`'s single-note output. `---`
  was picked because it can't collide with the `# notetype:`/`## FieldName`
  header syntax; `escape_field` now also backslash-escapes a field body
  line that's exactly `---`, the same way it already escapes `## `-prefixed
  lines, so that content round-trips through `show`/multi-note `add`
  unchanged.
- That escaping only protects notes dumped *after* the delimiter existed.
  A note file dumped by an older `cards show` could have an unescaped
  `---` sitting in a field's own content, so `split_note_blocks` only
  treats a `---` line as a boundary when it's immediately preceded by a
  blank line (or is the file's first line) — the same position every `## `
  header occupies. That doesn't make the delimiter unambiguous (a `---`
  that was already isolated on its own paragraph in old prose is
  indistinguishable from a deliberate separator, and can't be made so
  without a different, uglier delimiter), but it does mean a `---` that's
  merely part of a running line of text — the common case — round-trips as
  content instead of silently truncating the field at that point. Pinned
  by `a_delimiter_line_not_preceded_by_a_blank_line_is_kept_as_field_content`
  in `cards.rs`.
- `parse_note_files_for_add` errors on an input file with zero notes in it
  (empty, whitespace-only, or every block within it blank) instead of
  quietly returning `Ok(vec![])`: `parse_note_file` already refused this
  for the single-note case ("no `## FieldName` sections found"), and
  `cards::add_many` iterating zero notes would otherwise print nothing and
  exit 0 for what's almost always a bad file, not an intentional no-op.
- `cards::add_many` validates every note in a multi-note batch (its
  `# notetype:` header against `--notetype`, its fields against the
  notetype's own field names) *before* adding any of them: this is the same
  typo-safety guarantee single-note `add` always had, just extended so one
  bad note can't leave the batch half-applied. Once every note in the batch
  has passed that check, each is added independently via `Collection::
  add_note`, and its own success (note id, card count) or failure is
  reported without aborting the rest of the batch — that second failure
  mode is for genuine per-note rslib content requirements (e.g. a Cloze
  note missing its `{{c1::}}` deletion) that only surface at add time and
  have nothing to do with the other notes in the file.
- `notetypes list`/`notetypes show` (`notetypes.rs`) mirror `decks.rs`'s
  functional-core style: `Collection::get_all_notetypes`/
  `get_notetype_by_name` do the I/O, a pure function renders the result.
  `list` sorts by name for deterministic output, since
  `get_all_notetypes`'s order isn't documented by rslib. `show`'s field
  list is exactly `Notetype::fields`'s declared order — the same order
  `cards add --field`/the note-file format require — pinned against a real
  in-memory collection the same way `indents_nested_decks_by_level` pins
  the deck tree, since rslib has already been mis-modeled here once.
- `decks add` deliberately doesn't reuse `get_or_create_normal_deck`'s
  "return the existing deck" behavior for an exact-name collision: it calls
  `get_deck_id` first and errors if the deck already exists, since a
  command whose whole point is "make a new deck" silently no-op'ing on an
  existing one is the same kind of typo trap `cards add`'s
  deck/notetype-must-already-exist check guards against, just inverted.
  Missing parent segments of a `::`-nested name (e.g. `Spanish::Vocab` when
  only `Spanish` exists) are still created automatically — that's rslib's
  own `match_or_create_parents`, invoked by `add_deck_inner` for any new
  deck, nested or not, and is the same behavior Anki's own deck picker
  relies on.
- `cards sync` (`sync.rs`) makes a note file the source of truth: blocks
  with a `# nid:` update that note (fields, tags, and moving its cards to
  the block's `# deck:`), blocks without one are added and get their new
  nid written back. `# notetype:`/`# deck:`/`# tags:` omitted from a block
  are inherited from the first block, so a file's top reads as its header;
  `# nid:` never is. Notes removed from the file are deliberately left in
  Anki — deleting from a missing block would turn a bad merge or a
  truncated file into silent data loss.
- Write-back goes through `note_block_spans` (byte ranges into the original
  text) rather than re-rendering the file, so the only bytes that change
  are the inserted `# nid:` lines (once `read_text` has made it LF).
  `split_note_blocks` is now a thin wrapper over the same spans, so
  `cards add` and `cards sync` can't disagree about where blocks are.
- `sync` keeps `add_many`'s two-phase shape (validate everything, then
  write each note independently) and adds nid checks to phase one. Missing
  decks under `--create-deck` are only created after validation passes, so
  a file that fails validation creates nothing. "Unchanged" is decided by
  comparing fields and tags before calling `update_note` (rslib also skips
  no-op updates itself, but doesn't report that back).
- `sync` (`ankiweb.rs`) is rslib's `Collection::normal_sync`, nothing more:
  a two-way merge in one transaction that rslib rolls back on error. When
  AnkiWeb answers `FullSyncRequired`, rslib hasn't changed anything, and we
  error rather than pick a direction (`full_upload`/`full_download` can
  each discard the other side's changes). Media isn't synced. Building
  needs `protoc` (for `anki_proto`), and HTTPS needs the `anki` crate's
  `rustls` feature: rslib's reqwest is `default-features = false`, so
  without it everything compiles but every request fails at runtime.
- Credentials come from desktop's `prefs21.db` (`prefs.rs`), read-only:
  each profile's row is a pickled dict, decoded with `serde-pickle`'s
  `replace_unresolved_globals` because it can hold PyQt objects pickled via
  `sip`. We never write it back (re-pickling would drop those values), so
  a `new_endpoint` redirect from AnkiWeb is only reported; desktop gets the
  same redirect on its own next sync. Key names mirror
  `qt/aqt/profiles.py` and can change with any desktop release.
- `ankiweb.rs`'s test runs a real normal sync against rslib's own
  in-process sync server (`SimpleServer::make_server`), which reads its
  user from the `SYNC_USER1` env var; that's the only test allowed to touch
  the environment.
- `notetypes edit`/`notetypes add --input` read `show --full` output with
  the same `cards::split_sections`/`escape_field`/`trim_field` and fixed
  two-newline boundary as note files. Unlike note fields, rslib keeps `\r`
  in templates and CSS, which `str::lines()` can drop, so
  `apply_notetype_file` compares each section with what `show` prints for
  it, read back the same way, and keeps the stored bytes when they match:
  only sections actually edited are rewritten, and only those lose a `\r`.
  (A template *name* containing `\r`, which only the API can create, makes
  `edit` error out.) The last section is the one an editor's "trim trailing
  blank lines" can damage, so when its boundary is missing its stored
  trailing newlines are kept. `--full` escapes the name lists; plain `show`
  prints them verbatim.
  Use `show --output` rather than shell redirection, which can re-encode
  the file (Windows PowerShell 5.1 writes UTF-16).
- Both commands require the file's field and template lists to match the
  notetype exactly. In rslib, `update_notetype` calls `set_schema_modified`
  (which forces a one-way full sync) only from `schemachange.rs`, when field
  or template ordinals are added, removed or reordered, or the sort field
  moves. Template HTML and CSS edits don't trigger it. `add_notetype` never
  does: a new notetype goes up in a normal sync. Desktop's Clone
  (`models.copy`) doesn't call `mark_schema` either. Pinned by
  `template_and_css_edits_and_clones_leave_the_schema_unmodified`, which
  reads `sync_meta().schema` and uses a template reorder as its positive
  control.
- `notetypes add` checks the name is free first, because rslib's
  `ensure_notetype_name_unique` would otherwise append `+` without saying
  so. It checks the name rslib will store (`"` stripped, NFC), not the one
  typed. The clone copies what desktop's `models.copy` does: fresh id,
  `original_id` cleared, everything else kept.
- `cards suspend/unsuspend/bury/unbury/flag` (`card_state.rs`) act on
  cards, not notes: in rslib these are card state (queue and flags), so a
  note's cards are targeted with `--search nid:...`. They take card ids
  (each checked to exist) or a non-empty `--search`. `unsuspend`/`unbury`
  first narrow to `is:suspended`/`is:buried`, because rslib's
  `unbury_or_unsuspend_cards` restores both states and would otherwise
  unbury on `unsuspend` (and vice versa). `bury` is `BuryUser`, which is
  what desktop's manual bury uses; rslib won't bury suspended cards.
