# Rust dependency governance

Rustle treats its resolved Cargo graph as production infrastructure. New direct
dependencies and policy exceptions require a named purpose, an owner, a feature
review, an exit condition, and a rollback path.

## Authoritative checks

The repository pins cargo-deny in `.cargo-deny-version`. Install that exact
version and run the xtask-owned policy command:

```bash
cargo install cargo-deny --version 0.20.2 --locked
cargo xtask supply-chain
```

`deny.toml` checks every workspace member with all features. Xtask evaluates
the supported Windows, Linux, and macOS targets separately so target-specific
transitive dependencies do not leak across an artificial union graph.
Advisories, licenses, wildcard version requirements, registries, and git
sources are policy inputs; Cargo.lock remains the exact resolved dependency
record.

The desktop package is explicitly `publish = false`. Cargo-deny therefore
permits revision-pinned git/path dependencies while still rejecting wildcard
registry versions; Rustle is an application artifact, not a crates.io library.

## Git source register

Every direct git dependency in Cargo.toml uses a full `rev`. Some transitive git
sources are selected by the pinned Iced fork and are registered here because
cargo-deny evaluates the complete lockfile graph.

| Repository | Current revision | Role and owner | Upgrade and rollback |
| --- | --- | --- | --- |
| `Fei-xiangShi/iced` | `573ebbf00ed3f544e784488cd3e71ab0ea10ec1b` | Rustle maintainers own the GUI/runtime fork needed by the current Iced 0.15 development integration. | Compare with upstream Iced, update the manifest rev and lockfile together, then run the full xtask and native CI matrix. Roll back to this reviewed rev. |
| `Fei-xiangShi/client-toolkit` | `f69a98395ead55777832525623ff82d40c16a25b` | Rustle maintainers own the Wayland/client-toolkit compatibility fork selected by the GUI stack. | Upgrade independently from application dependencies where possible and verify Linux native startup/tray behavior. Roll back the patch rev and lockfile. |
| `Fei-xiangShi/winit` | `b9559736198002515db605268c4e0065246ef70e` | Transitive source owned through the pinned Iced fork; Rustle maintainers own compatibility validation. | Move only with the owning Iced revision, verify all native runners and window/protocol behavior, then retain the previous Iced/lockfile pair as rollback. |
| `iced-rs/cryoglyph` | `f4e7e4eb84dc1d2f335a98c67d8694640d53a433` | Transitive text renderer selected by Iced; the GUI stack owner reviews it. | Move through a reviewed Iced update and verify text shaping, lyrics rendering, GPU startup, and native CI. Roll back the owning Iced revision. |
| `SPlayer-Dev/ncm-api-rs` | `133b65bfe482e41ebccf018870d3fce07bf58eb3` | The NCM adapter owner uses this as the single protocol implementation. | Review endpoint/crypto/cookie changes and lockfile deltas, run offline contract fixtures and full tests, then update the exact rev. Roll back to this revision if online behavior regresses. |

An allowed repository URL is not permission to follow a branch. New or updated
manifest entries still require a full commit revision. Workspace manifests use
one inline table per git dependency or patch; xtask rejects missing/non-40-digit
`rev` values and any simultaneous branch/tag selector. If a transitive source
disappears after an owning dependency upgrade, remove its allow entry in the
same change.

## Exception workflow

Current exact advisory exceptions are:

- `RUSTSEC-2023-0071`: `ncm-api-rs` uses RSA for public-key protocol
  encryption; the timing issue concerns private-key operations. The NCM adapter
  owner tracks [the RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071)
  and removes the exception when the adapter replaces RSA or upstream provides
  a constant-time release.
- `RUSTSEC-2024-0436`: `paste` is transitive through the media/image graph.
  The dependency-convergence owner tracks [the RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2024-0436)
  and removes it when the owning crates migrate to maintained macro support.
- `RUSTSEC-2025-0141`: `bincode` is present through Iced debug/devtools. Remove
  under the GUI dependency owner when production feature hygiene or an
  upstream serialization change removes that path; see [the RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2025-0141).
- `RUSTSEC-2026-0192`: `ttf-parser` is used by the current text stack. Replace
  it with maintained fontations-based APIs during dependency convergence; the
  text-stack owner tracks [the RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2026-0192).

`ncm-api-rs 0.1.0` also has an exact WTFPL license exception; this does not add
WTFPL to the global accepted license set.

- Advisory ignores use an exact RustSec ID and include impact, owner, upstream
  tracking link, and an expiry or concrete removal condition.
- License exceptions apply to an exact crate/version range and cite the license
  evidence. Do not add a license to the global allow set for one package.
- Unknown registry or git-source failures are not bypassed with organization-
  wide allow rules. Register only the reviewed canonical repository.
- Duplicate-version warnings are baseline evidence for the dedicated version-
  convergence task. GPU, windowing, platform, and audio ecosystems may retain
  justified parallel versions; do not force convergence solely to reduce a
  count.

Policy failures may be temporarily excepted only in the same reviewed change
that records an owner and removal condition. Disabling the complete CI job or
using `continue-on-error` is not an acceptable exception.
