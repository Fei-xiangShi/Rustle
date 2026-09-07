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
- other user tables without a ledger: compute a metadata-only structural
  fingerprint. Exact released v1–v5 schemas use the temporary legacy
  initializer without claiming a ledger; unknown/partial schemas stop before
  DDL with `storage.schema_unsupported`.

The final branch will remove the legacy route after backup/recovery and adoption
are complete. New schema changes must already use a new numbered SQL migration;
do not add DDL to `src/database/schema.rs`.

## Released legacy fixtures

Reviewable cumulative SQL fixtures live under `tests/fixtures/database/`:

| Identity | Released versions | Change |
| --- | --- | --- |
| V1 | v0.1.0–v0.2.2 | Initial library/playlist/queue/history/watched-folder schema. |
| V2 | v0.2.3 | Normalization gain. |
| V3 | v0.2.4–v0.3.2 | Missing-state and watched-folder playlist ownership. |
| V4 | v0.3.3–v0.4.6 | Download history. |
| V5 | v0.4.7–v0.5.2 | Personal-FM playback state. |

The classifier canonicalizes structural metadata instead of hashing raw DDL.
Column order is ignored because ALTER-upgraded and freshly created databases
can differ; type/default/nullability/PK semantics, foreign-key actions, indexes
and partial predicates, checks, tables, views, and triggers remain significant.
Fixture hashes are schema identities, not security primitives, and contain no
user content.

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
canonical-schema parity, checksum rejection, all five historical upgrades,
data preservation, ALTER/fresh equivalence, and strict structural mutation
rejection. The full `cargo xtask check`, native gate, and supply-chain policy
remain required before merging database infrastructure.

Never validate migration code against a developer's live Rustle database.
Historical upgrade tests use pristine repository fixtures, and destructive or
irreversible adoption work requires a verified backup and restore path first.
