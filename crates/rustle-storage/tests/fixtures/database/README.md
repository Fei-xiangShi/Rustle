# Rustle historical SQLite fixtures

These scripts reconstruct every distinct schema generation shipped by Rustle.
They are cumulative: build generation Vn by applying `legacy_v1.sql` followed
by every `legacy_vN_delta.sql` up to the requested generation.

| Generation | Source commit | Released tags | Files applied |
| --- | --- | --- | --- |
| V1 | `4ad087e134e619f8439ded85ae765438f045dc46` | v0.1.0–v0.2.2 | V1 |
| V2 | `bd30afa2ae4eae94f01de904b7b1f18ee3e2519a` | v0.2.3 | V1–V2 |
| V3 | `394f35871bd82f547a8ee7f2cabe1d964f71f711` | v0.2.4–v0.3.2 | V1–V3 |
| V4 | `f9ed27fcae58cd78a44bf4cded6a68c19d40609b` | v0.3.3–v0.4.6 | V1–V4 |
| V5 | `b02a5304c009d68d01b8a705fa7f5f7f639dacf5` | v0.4.7–v0.5.2 | V1–V5 |

The rows are synthetic and intentionally use relative fixture paths. Never
replace these inputs with a copied developer/user database. V5 is the current
unversioned canonical application schema; the SQLx-managed baseline adds only
its migration ledger.

## Adoption dependency

These fixtures authorize classification only. They must not be used to claim a
SQLx migration version until the adoption path has created and verified a
recoverable pre-mutation backup, normalized the recognized V1–V5 schema,
installed the exact baseline ledger, run startup health checks, and proved
restore behavior for failed or interrupted adoption. Unknown fingerprints are
never repair candidates.
