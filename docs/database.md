# Rust database engineering

Rustle stores user library, playlists, queue, playback state, history, watched
folders, and download history in SQLite through SQLx. Schema changes are user
data migrations and must remain versioned, testable, and recoverable.

## Current migration and adoption model

`migrations/0001_canonical_schema.sql` is embedded into the binary with
`sqlx::migrate!`. A database is classified at startup without reading user
rows:

- no user tables: run the embedded migration and create the SQLx ledger;
- `_sqlx_migrations` present: validate checksums and run pending migrations;
- other user tables without a ledger: compute a metadata-only structural
  fingerprint. Exact released v1–v5 schemas enter the verified adoption path;
  unknown or partial schemas stop before DDL with
  `storage.schema_unsupported`.

Legacy adoption closes the ordinary application pool, then runs on a dedicated
maintenance thread with its own current-thread Tokio runtime. This keeps raw
SQLite DDL out of Iced's `Send` task boundary and releases every ordinary pool
handle before Windows file recovery. After successful adoption, the repository
opens a fresh policy-configured application pool.

`src/database/schema.rs` no longer exists. Reviewed V1-to-V5 compatibility SQL
lives under `migrations/legacy/`; it is only the one-time path into immutable
SQLx migration `0001`. Every future product schema change must use a new
numbered SQLx migration.

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

## Pre-adoption backup and recovery

Exact legacy databases are backed up before any compatibility DDL. The sibling
`database-backups/` directory contains strict owned pairs:

```text
rustle-pre-adoption-v<1..5>-<unix_ms>-<process_id>-<counter>.sqlite3
rustle-pre-adoption-v<1..5>-<unix_ms>-<process_id>-<counter>.manifest.json
```

The storage owner preflights caller-available filesystem space through `fs4`,
then uses SQLite `VACUUM INTO` so the standalone backup includes committed rows
that are still visible through WAL. Before source mutation, the backup is
opened read-only and must pass `quick_check`, foreign-key verification, exact
typed legacy identity, byte-size verification, and a streaming XXH3-128
digest. Its versioned JSON manifest contains only application/schema/migration
identity, timestamp, size, and digest; it contains no database path, SQL, or
user rows.

Adoption takes `BEGIN IMMEDIATE`, rechecks the exact source identity, applies
only the reviewed legacy deltas, verifies canonical V5, and inserts migration
0001 into the SQLx ledger using the version, description, and checksum embedded
in the binary. Normalization and ledger ownership commit atomically. SQLx then
runs pending migrations and health checks require checksum validity, managed
classification, SQLite integrity, no foreign-key violations, and the playback
singleton invariant.

If a failure occurs after commit, the pool is closed and the published
backup/manifest pair is reverified against the embedded baseline. Recovery
copies and flushes a new temporary file, removes only the database's exact WAL
and SHM sidecars, atomically replaces the live file, and confirms the restored
legacy identity. The verified backup is retained. After success, retention
keeps the current backup and at most three verified owned pairs in total;
foreign, incomplete, unmatched, or corrupt files are never pruned.

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

Backup/adoption failures additionally expose stable privacy-safe codes:

- `storage.backup_failed`;
- `storage.insufficient_space`;
- `storage.adoption_failed`;
- `storage.recovery_failed`.

`fs4` is owned only by the storage adoption preflight. It replaces handwritten
platform capacity code and can be removed when the Rust standard library offers
an equivalent cross-platform caller-available-space API.

## Validation

The focused local gate is:

```bash
cargo test --locked --lib database -- --test-threads=1
```

Tests cover every pooled connection, empty-to-latest, repeated latest startup,
canonical-schema parity, checksum rejection, all five historical upgrades,
data preservation, ALTER/fresh equivalence, strict structural mutation
rejection, WAL-visible backup rows, insufficient space, backup publication
failure, transaction rollback, concurrent schema change, post-commit automatic
restore, corrupt backup/manifest rejection, read-only verification, and strict
owned-name retention. The full `cargo xtask check`, native, production, and
supply-chain gates remain required before merging database infrastructure.

Never validate migration code against a developer's live Rustle database.
Historical upgrade and recovery tests use pristine synthetic repository
fixtures only.
