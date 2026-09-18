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
