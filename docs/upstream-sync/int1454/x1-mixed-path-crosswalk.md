# X1: retired-tagged commits with surviving root paths

Of 2,700 commits tagged `retired` in [the frozen inventory](generated/commits.tsv), 85 touch 22 names that still exist at frozen upstream `e151d5ea`. All 85 entered through the Solidcore history parent `916b91e8d^2`; none are ancestors of the app-side parent `916b91e8d^1`. At the import, `916b91e8d:solidcore` equals the second parent's tree. A surviving root name does not alone establish an application change to port.

| Owner | Surviving root paths | Evidence and outstanding work |
| --- | --- | --- |
| K0 / CD-7 | Vendored counterparts of 21 of the 22 names | `e151d5ea:solidcore` and `db0b8dd35:solidcore` both have tree `8b5c01445619305dcfc5072db3fe76b8cfea67be`. This establishes the nested source snapshot, not runtime coverage or root workflow parity. |
| B1 / K0 / K12 | `.github/workflows/{ci,pages,release}.yml` | `db0b8dd35:.github/workflows/kernel-rig.yml` is configured to run selected IntCAD case and ladder checks via `scripts/ci/kernel_rig_plan.py`; this is a separate product rig, not the upstream vendored goldens/profile jobs (`96b1a9869`). The nested kernel CI retains those jobs and `check_panic_sites.py` (`7236f4410`); no corresponding root workflow invocation was identified at `db0b8dd35`. Source presence is not execution parity. |
| B3 / B1 | `scripts/ci/warm-target.sh` | The only name without a nested counterpart. The root blob `6bbb3aedccb10b740956809e27524303e257b55f` matches at `e151d5ea`, `0fd92bea2`, and `db0b8dd35`; no workflow caller was identified. Source presence is not a test receipt and does not fall under #203 or standalone enforcement. |
| B6 / K0 | Root `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, `.gitignore`, `.gitattributes`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`, `rustfmt.toml` | See the [root Cargo workspace evidence](x1-b6-root-workspace.md) for the manifest, lockfile, and deny policy dispositions; see below for ignore and attribute rules. The root `.cargo/config.toml`, `clippy.toml`, and `rustfmt.toml` match the frozen source at baseline and candidate. #180 covers the Rust pin and its CI wiring only. |
| B8 / K15 / K0 | `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `ATTRIBUTION.md`, `NOTICE`, `assets/README.md`, `docs/{layers.md,mutation-testing.md,sponsors/blacksmith-ci-powered.png}` | The [root provenance crosswalk](x1-b8-root-provenance.md) pins all nine paths, separates root guidance and credits from nested notices, and records open K15, rights and packaging decisions. |

To reproduce the source classification, select exact comma-delimited `retired` tags in the frozen TSV, list paths changed by those SHAs with `git diff-tree --stdin -r -M --name-only --format=@@%H`, and intersect with `git ls-tree -r --name-only e151d5ea`. Use `git rev-parse <revision>:<path>` to compare pinned source objects. This is an owning-row crosswalk, not an adoption receipt or a whole-commit exclusion.

## Root Git rules

The candidate `db0b8dd35` retains the pre-sync `0fd92bea2` root `.gitignore`
(`a148f97e639b`) and `.gitattributes` (`0ff3cca6dcfa`). Frozen upstream
`e151d5ea` has different blobs (`10fd26817f26` and `9899538358e3`).

| Rule | Owner and disposition |
| --- | --- |
| Ignore root `/node_modules/` | B6: preserve the IntCAD rule. The remaining upstream-only `.gitignore` delta is a plugin-example comment, not another pattern. |
| Pin the icon discovery list to LF | B8 / CD-6: candidate pins `assets/icons/ui/design/requested_icons.txt`; upstream pins its own `crates/look/` path. Keep the IntCAD path; no brand import. |
| Pin `*.wat` to LF | A3 / CD-5: upstream has six tracked plugin fixtures; candidate has zero tracked `.wat` files. Do not import the rule as a plugin migration deliverable while the plugin host is deferred. |
| Pin `*.wgsl` to LF | B6 / U3: **open**. Candidate has 20 tracked WGSL files, including shaders loaded via `include_str!`. `git check-attr --source=db0b8dd35` reports `eol: unspecified` for the tessellation shader, while frozen upstream reports `eol: lf`. The focused probe below reproduces a candidate assertion failure under CRLF; the native Windows GPU test still needs verification. |
| Mark `solidcore/**` as `linguist-vendored` | B6 / K0: upstream-only GitHub language statistics rule. Candidate does not set it. Any adoption is separate from CD-7, licence coverage, and building the nested workspace. |

No Git configuration, checkouts, or product files were changed; no shader
test or Windows run is claimed.

### Focused WGSL line-ending probe

Upstream commit `98fdb6ef3` introduced the WGSL LF rule for multiline
assertions. At `db0b8dd35`, `crates/gpu/src/path/volume_tests.rs` test
`gpu_volume_signed_primary_preserves_old_perspective_lens_none_and_zero_bytes`
matches two exact multiline literals against a shader assembled by
`path_shader!` in `crates/gpu/src/path/mod.rs`. The exact test name is absent
at `1b7255797` (the parent of current `develop` `79bf25d89`; #203 changed
only two CI/test scripts). This probe does not establish a current
`develop` failure.

On Linux, Git's checkout filters (`git --attr-source=<pin> -c core.autocrlf=true
cat-file --filters db0b8dd35:<path>`) supplied the same ten candidate shader
blobs under either candidate or frozen-upstream attributes. A disposable
Rust 1.99.1 probe used the candidate's shader assembly macro and its two
unchanged match-count assertions. With candidate attributes `volume.wgsl`
had 143 CRLF lines, both counts were zero for the first producer, and the
probe exited `101`. With upstream attributes it had 143 LF lines, and both
counts were one for each producer (exit `0`). No GPU, Cargo test suite,
Windows checkout, or integration PR was run or changed.

Local receipts: `/tmp/opencode/int1454-wgsl.XVXlq6/{crlf,lf}/run.{log,exit}`.
For either variant, with `d` set to the receipt directory and `mode` set to
`crlf` or `lf`, compile the saved probe with
`rustc "$d/$mode/probe.rs" -o "$d/$mode/probe"` (Rust 1.99.1), then run
`"$d/$mode/probe"`. The probe embeds the variant's ten adjacent shader files
at compile time via `include_str!`; the saved run logs do not record the
compiler command.
Treat the LF rule as a candidate portability change for separate approval;
retain the existing assertions and confirm the native GPU test before
claiming Windows parity.
