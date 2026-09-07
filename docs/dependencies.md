# Rust dependency governance

Rustle treats its resolved Cargo graph as production infrastructure. New direct
dependencies and policy exceptions require a named purpose, an owner, a feature
review, an exit condition, and a rollback path.

## Authoritative checks

The repository pins cargo-deny and cargo-machete in root version files. Install
those exact versions and run the xtask-owned policy command:

```bash
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-machete --version 0.9.2 --locked
cargo xtask supply-chain
cargo xtask check-production
```

`deny.toml` checks every workspace member with all features. Xtask evaluates
the supported Windows, Linux, and macOS targets separately so target-specific
transitive dependencies do not leak across an artificial union graph.
Advisories, licenses, wildcard version requirements, registries, and git
sources are policy inputs; Cargo.lock remains the exact resolved dependency
record.

The same command runs cargo-machete over repository source while skipping build
output. Machete's metadata-assisted mode currently misclassifies the cfg-gated
Windows build dependency used by `build.rs`, so the enforced invocation uses
the source scanner and requires manual confirmation before deleting a reported
dependency.

Cargo-deny rejects new parallel versions by default. The exact older versions
that remain are individually registered in `[bans].skip`, with an ownership
reason and no wildcard ranges. Because the policy is evaluated one native
target at a time, xtask suppresses only `unmatched-skip` and
`unnecessary-skip` diagnostics caused by entries that belong to another target;
duplicate-version findings remain denied.

The desktop package is explicitly `publish = false`. Cargo-deny therefore
permits revision-pinned git/path dependencies while still rejecting wildcard
registry versions; Rustle is an application artifact, not a crates.io library.

## Production features

The default build excludes Iced's debug/devtools graph. Developers can opt into
it explicitly when investigating renderer or widget behavior:

```bash
cargo run --features devtools
```

`cargo xtask check-production` compiles the default workspace graph and rejects
`iced_devtools` or `iced_beacon` in normal production dependencies. The full
quality gate still uses all features so the opt-in developer path remains
compiled, linted, and tested.

The `zip` dependency is owned by the user-invoked diagnostic exporter and is
optional behind the non-default `diagnostics` feature. The default desktop
graph must not contain it. It uses only the reviewed deflate backend; expanding
compression or crypto features requires a new size, license, and supply-chain
review. Remove the dependency if diagnostics move to a separately distributed
tool or a standard-library-compatible archive format becomes sufficient.

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
  it when the owning font/render crates move to a fixed release; the direct
  Rustle declaration has already been removed. The text-stack owner tracks
  [the RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2026-0192).

`ncm-api-rs 0.1.0` also has an exact WTFPL license exception; this does not add
WTFPL to the global accepted license set.

- Advisory ignores use an exact RustSec ID and include impact, owner, upstream
  tracking link, and an expiry or concrete removal condition.
- License exceptions apply to an exact crate/version range and cite the license
  evidence. Do not add a license to the global allow set for one package.
- Unknown registry or git-source failures are not bypassed with organization-
  wide allow rules. Register only the reviewed canonical repository.
- Duplicate versions are deny-by-default. A retained version needs an exact
  `[bans].skip` entry with an owner and exit condition; GPU, windowing,
  platform, font, image, network, and audio ecosystems must not be forced into
  one API generation solely to reduce a count.

Policy failures may be temporarily excepted only in the same reviewed change
that records an owner and removal condition. Disabling the complete CI job or
using `continue-on-error` is not an acceptable exception.

## Reviewed duplicate baseline

At baseline commit `0fee60d`, the lockfile contained 820 packages, 58 names
with parallel resolved entries, and 48 direct dependency declarations. The
dependency-convergence change removes `iced_anim`, `iced_core 0.14`,
`glam 0.25`, and the unused direct `ttf-parser` declaration. The resulting
graph contains 817 packages, 56 duplicate names, and 46 direct declarations.
`ttf-parser` remains only as a transitive font-stack dependency.

The exact accepted versions live in `deny.toml`; the ownership and removal
conditions are grouped here so future upgrades have a clear route:

| Family | Current owner and why it remains | Exit condition |
| --- | --- | --- |
| Reqwest 0.12/0.13 | The pinned `ncm-api-rs` revision owns Reqwest 0.12 while Rustle uses 0.13. | Review an upstream/fork adapter revision, verify protocol/cookie/error contracts, then update its exact git rev. |
| Windows bindings and target crates | Native, tray, media, and windowing crates span several ABI generations and target support packages. | Upgrade the owning native crates by platform and remove each exact skip only after all native CI targets pass. |
| Fontations (`font-types`, `read-fonts`, `skrifa`) | Iced/renderer/font crates follow separate coordinated release trains. | Move through a reviewed renderer/text-stack upgrade with shaping, lyrics, and GPU validation. |
| Image/render (`png`, `tiny-skia`, `kurbo`, compression) | SVG, raster image, and renderer crates require semver-incompatible APIs. | Upgrade the owning image/SVG stack and verify decode, cache, cover, and renderer tests. |
| Randomness (`rand`, `rand_core`, `getrandom`, `r-efi`) | Crypto, media, and platform crates consume three API generations. | Remove older versions as their owning transitive crates publish compatible releases. |
| Macro/build/TOML (`syn`, `thiserror`, `proc-macro-crate`, `toml*`, `winnow`) | Proc macros and platform build tooling have not converged on one parser generation. | Upgrade the owning macro/build crates; do not patch semver-incompatible parser APIs globally. |
| Platform adapters (`x11rb`, `jni`, `objc2`, `keyboard-types`, `core-foundation`) | Desktop and optional platform backends require distinct adapter generations. | Converge per owning backend after target-specific compile and runtime smoke checks. |

Adding a new duplicate must fail `cargo xtask supply-chain`; growing the
baseline requires a reviewed exact entry and documentation in the same change.
