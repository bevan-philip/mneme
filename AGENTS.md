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
- Backups are written under a `<fingerprint(anki2_root)>-<profile>`
  subdirectory, not just `<profile>`: rslib's thinning treats every file in
  the directory it's given as one collection's retention history, so two
  different `--anki2-root`s sharing a profile name would otherwise prune
  each other's recovery points too. `paths::fingerprint` is a plain FNV-1a
  hash of the (lexically absolute) root path — no new dependency, no need
  for the path to exist yet.
