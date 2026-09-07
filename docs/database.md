# Rust database engineering

Rustle stores user library, playlists, queue, playback state, history, watched
folders, and download history in SQLite through SQLx. Schema changes are user
data migrations and must remain versioned, testable, and recoverable.

## Current staged migration model

`migrations/0001_canonical_schema.sql` is embedded into the binary with
`sqlx::migrate!`. A database is classified at startup without reading user
rows:

- no user tables: run the embedded migration and create the SQLx ledger;
- `_sqlx_migrations` present: validate checksums and run pending migrations;
- other user tables without a ledger: use the temporary legacy initializer and
  do not claim a migration version.

The final branch will remove the legacy route after the historical-fixture and
backup/recovery waves are complete. New schema changes must already use a new
numbered SQL migration; do not add DDL to `src/database/schema.rs`.

## Connection contract

The shared connection factory applies the policy to every pooled connection:

- foreign keys enabled and verified;
- WAL journal mode;
- NORMAL synchronous mode;
- five-second busy timeout;
- 32 MiB SQLite cache budget;
- five maximum pooled connections.

Failure to apply this policy fails database initialization. Migration failures
retain SQLx's source and surface as the stable
`storage.migration_failed` diagnostic code.

## Validation

The focused local gate is:

```bash
cargo test --locked --lib database -- --test-threads=1
```

Tests cover every pooled connection, empty-to-latest, repeated latest startup,
canonical-schema parity, checksum rejection, and preservation of the current
unversioned schema/data route. The full `cargo xtask check`, native gate, and
supply-chain policy remain required before merging database infrastructure.

Never validate migration code against a developer's live Rustle database.
Historical upgrade tests use pristine repository fixtures, and destructive or
irreversible adoption work requires a verified backup and restore path first.
