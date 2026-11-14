# Releasing

This is the operational runbook. The branch model is in `BRANCHING.md`; the
version/feature rules are in `README.md`. Nothing here releases automatically
except the tag-driven `release` workflow.

## Version sources

| Repository | Version source |
|---|---|
| Pagh-OS, pagh-riscv, pagh-rust | `Cargo.toml` (`[package].version`; `pagh-rust` uses `pagh_std/Cargo.toml`) |
| **pagh-python (this repository)** | **`Cargo.toml`** (`[package].version`) |
| pagh-libc | `VERSION` |
| pagh-api | `libpagh/Cargo.toml` |
| pagh-programs | `programs/Cargo.toml` |
| pagh-repo | index manifest version |

`pythonc` is a single package, so the root `Cargo.toml` carries the version
(mirrored by `Cargo.lock`). `.github/workflows/release.yml` reads exactly that
file (`VERSION_FILE`).

`dev` carries the **next release** version. The first feature after a release
bumps MINOR, the first fix bumps PATCH. `main` keeps the last **released**
version. A stable tag `vX.Y.Z` must equal the version in `Cargo.toml` on `main`;
the README's «Рабочая версия» line and `CHANGELOG.md` are updated in the release
commit, but the manifest is the only machine-checked source of truth (this
repository has no `AGENTS.md`).

## Pre-release (release candidate) — from `dev` only

1. Freeze `dev`: after `vX.Y.Z-rc.1`, only fixes for this release are merged
   until the release lands. Everything else waits.
2. Pre-flight on the exact `dev` commit:
   - CI green — the required check is the `ci` job
     **host tests, build, image size**;
   - the local evidence below is green, with a fresh live QEMU transcript for
     the version being released;
   - `CHANGELOG.md` has an entry for `X.Y.Z` (it is the human-readable release
     note; GitHub additionally generates notes from commits);
   - version source equals `X.Y.Z` (no `-rc` in the source file);
   - no open blocker, no known regression in the release scope.
3. Tag and push `vX.Y.Z-rc.N`:
   ```sh
   git checkout dev && git pull --ff-only
   git tag -a vX.Y.Z-rc.1 -m "pagh-python X.Y.Z-rc.1"
   git push origin vX.Y.Z-rc.1
   ```
4. Watch the `release` workflow: it refuses the tag unless it is reachable
   from `dev` **and not** from `main`, checks the base version against
   `Cargo.toml`, writes the source archive `pagh-python-vX.Y.Z-rc.N.tar.gz` plus
   `.sha256`, and publishes a GitHub **Pre-release**. A rejected tag stays in
   the repository but produces **no** release entry: delete it
   (`git push origin :refs/tags/<tag>`) before retrying.
5. Test the pre-release source archive: unpack it, build it against the pinned
   toolchain and the sibling `pagh-api`, and run the host and live checks.
   Bugs: fix on `dev`, tag `-rc.2` (never move the `-rc.1` tag).

## Local evidence for a release (this repository)

The release workflow does not build the interpreter — it attaches the tagged
source archive; `pythonc` is built and size-checked in `ci.yml`. The pre-flight
below is what a release candidate must have green on its exact commit:

1. **Host interpreter checks** — the same `src/*.rs` compiled against a std
   `platform`, 44 unit tests at 0.9.2, including the evaluator-depth and
   generator-stack-headroom regressions from the security audit:

   ```sh
   cd hostcheck
   cargo test --locked
   cargo run -- --selftest     # the same grid as PASS/FAIL lines (44 PASS)
   ```

2. **Freestanding release build inside the kernel's 1 MiB image limit**:

   ```sh
   cargo build --release
   wc -c target/riscv64gc-unknown-none-elf/release/pythonc
   ```

   At 0.9.2 the ELF is 582 056 bytes (569 KiB); the kernel refuses images above
   1 048 576 bytes. `ci.yml` asserts the limit, so a green CI covers items 1–2.

3. **Live QEMU suite** — the acceptance artifact is a committed transcript, not
   a CI job (it needs a `pagh-riscv` checkout and QEMU). One command rebuilds
   the release ELF, stages a 64 MiB ext2 image, drives the commands in one boot
   and rewrites the transcript:

   ```sh
   sh tests/live/live092.sh            # writes docs/live-pythonc-092.txt
   PAGH=/path/to/pagh-riscv sh tests/live/live092.sh /tmp/live.txt
   ```

   At 0.9.2 the suite is 69 runs: 51 PASS plus 18 intentional diagnostics
   (syntax/uncatchable errors, recursion bombs, process-ABI checks). The PASS
   list and raw log are in `docs/live-testing.md` and
   `docs/live-pythonc-092.txt`; link the fresh transcript in the release PR.

4. **Regression against the previous release** — the whole 0.1.2-onward suite
   and the committed security PoCs must still pass; the released transcript is
   the evidence.

## Stable release — from `main`

1. Merge `dev` into `main` (merge commit; no code changes at this step):
   ```sh
   git checkout main && git pull --ff-only
   git merge --no-ff dev
   git push origin main
   ```
2. Wait for CI on `main` to be green.
3. Tag and push `vX.Y.Z` on the merge commit; the workflow checks that the
   commit is reachable from `main` and that `Cargo.toml` matches.
4. Verify in **Releases**: the new entry is a full release with the versioned
   source archive (`pagh-python-vX.Y.Z.tar.gz`) and its `.sha256` — this
   repository ships no binary artifact. GitHub marks the newest stable as
   **Latest**.
5. `dev` continues with the next version (first change bumps it).

## Hotfix — straight to `main`, no RC

Hotfixes are limited to **critical/high verified security findings** or a
release-blocking regression in a released version. Everything else rides `dev`.

1. Branch from `main`: `hotfix/<short-name>`.
2. PR to `main` (never to `dev`), full CI plus a targeted regression test/PoC
   for the bug. Get it reviewed.
3. Merge to `main`, then tag the PATCH release `vX.Y.Z+1` directly (no RC) and
   verify the release entry.
4. Merge `main` back into `dev` with `--no-ff`:
   ```sh
   git checkout dev && git pull --ff-only
   git merge --no-ff main
   ```
   Resolve version conflicts in favour of the larger version (dev stays ahead).
   `CHANGELOG.md` merges additively: keep both release sections.
5. CI on `dev` must be green after the merge-back.

## Cross-repository ABI change order

When a change touches the C/ABI boundary, release in dependency order and only
then update dependants:

1. `pagh-api` / `pagh-libc` (whichever changed; if both, `pagh-api` first when
   the SDK pins the libc header).
2. `pagh-rust` / `pagh-python` (they consume `pagh-api`/`pagh-libc`).
3. `pagh-programs` (apps built against the new SDK), `pagh-repo` (published
   packages), and finally Pagh-OS / pagh-riscv images that ship the new
   runtime/ABI.
4. Update the pinned sibling versions (`Cargo.toml` path/registry deps,
   `Cargo.lock`) in the same release train, not later.

Never tag a dependant before its dependency is tagged; the release workflow
only checks branch/version, not cross-repo compatibility.

## Backfill and "Latest"

GitHub marks **Latest** by release creation time, not by version number. When
backfilling already-released versions, create tags in ascending version order
(oldest first), so the newest version is created last and becomes Latest.

## Rollback / mistakes

- Wrong tag on the wrong commit: delete the release (`gh release delete <tag>
  --yes`) and the tag (`git push origin :refs/tags/<tag>`), fix, retag.
- Workflow rejected the tag: delete the tag; no release exists to clean up.
- Bad stable release: prefer a PATCH hotfix over moving/deleting a published
  tag; if the release is minutes old and nothing depends on it, delete
  release + tag, then re-tag the fixed commit.
- Never move an existing tag; immutable tags are enforced by the tag ruleset.

## Release checklist (paste into the release PR / notes)

- [ ] Version source updated and equal to the tag base (`Cargo.toml`)
- [ ] `CHANGELOG.md` entry and README «Рабочая версия» line written
- [ ] CI green on the exact commit (`host tests, build, image size`)
- [ ] Live evidence run and linked (hostcheck + fresh QEMU transcript)
- [ ] Freeze respected (no feature merges after `-rc.1`)
- [ ] Source archive + `.sha256` downloaded, `sha256sum -c` clean, and the
      unpacked tree builds (`cd hostcheck && cargo test --locked`, then
      `cargo build --release`)
- [ ] `main`/`dev` synchronized per the flow above
- [ ] Release entry has the right type (Pre-release vs Release)
