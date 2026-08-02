# Gate 0 — what two adversarial design rounds found, and why no code shipped

**Date:** 2026-08-02 · **Status:** design record; implementation deliberately deferred · **Supersedes:** nothing

`docs/handoffs/2026-08-02-five-layers-a-ledger-and-why-everything-is-tier-3.md` puts Gate 0a–0f
first, because §9.6 makes Tier 2 conditional on "Gates 0–2 pass" and only 0g exists — so every
candidate is Tier 3. This is the design work for it. Twenty-six agents over two rounds produced
**zero designs judged sound**, and that is the result rather than a failure of the exercise: what
they produced instead is a corpus of verified corrections to §9.3, and the reason the code is not
written yet.

---

## 1. The contract, decided and not open

Round 1 produced six independent designs. **Four of six treated "could not check" as "passed"** —
§6.20's cardinal sin, and the same bug this project shipped a day earlier when `GateState` carried
booleans and `judged explain` reported Tier 2 for gates it had never run. Six agents, each told
explicitly that this was the sin to avoid, walked into it anyway.

That is not carelessness. It is the default shape of the problem, and prose does not fix it. So it
became a fixed contract for round 2, and it binds the implementation:

1. **Three-state probes.** `Refuses(Finding) | Clear | Unreadable(Reason)`. `Unreadable` never
   collapses into `Clear`. No `bool` for "did this conjunct fire", and no `Result` as the only place
   unreadability lives — a caller must not be able to get a safe-looking value out of a failed probe.
   The crate already has the right carrier: `ledger::Outcome` (`Satisfied | Failed | NotEvaluable`).
2. **Never launder a symlink.** No API may resolve a link and hand the target to a caller that could
   act on it. Containment re-resolves at every component, and an *intermediate* dangling link is its
   own state, not "contained".
3. **Ordinary repositories must pass.** A gate that refuses everything measures exactly as much as
   one that refuses nothing. Every conjunct states what it does on this repository and on a plain
   clone.
4. **Do not trust the spec's file paths.** See §3.

---

## 2. A bug in shipped code, found on the way

`crates/judged-core/src/gate1/state.rs:868` probes `root.join(".git/annex")` and `.git/lfs` to
detect a git-annex or LFS store. In a **linked worktree or a submodule, `.git` is a regular file**,
not a directory — verified here: `git worktree add` produced a 63-byte `.git` file. So the probe is
silently false in exactly those layouts, and Gate 1's §6.13 store detection is blind there. It must
use `git rev-parse --git-common-dir`.

Fixed, with `Repo::common_dir()`, in `fix/gate1-store-probe-common-dir`.

**The same bug class is in seven more places, and all of them are still shipped.** Every walker in
the crate skips only a directory literally *named* `.git`, so each descends into a nested clone, a
linked worktree, a submodule and a bare `vendor/foo.git/`:

| file:line | walker |
| --- | --- |
| `gate1/state.rs:1016` | the Gate 1 state survey |
| `gate1/content.rs:1937` | the Gate 1 content scan |
| `gate3f.rs:654` | Gate 3f's marker scan |
| `roots/insource.rs:501` | the §5.2 in-source root scan |
| `roots/convention.rs:83` | Tier B convention detection |
| `roots/manifest.rs:2168` | Tier A manifest discovery |
| `veto/reachability.rs:129` | Gate 2b/2c directory enumeration |

Consequence varies by gate — a root materialized from a nested repository's manifest, a Gate 2
reference found in a vendored clone, an in-source marker read out of a submodule — but the shape is
one wrong assumption about git's layout, copied seven times.

**Fixed.** `judged_core::boundary::classify` is the shared predicate and all seven now call it. It
recognises all three `.git` shapes plus a bare repository (a widening of 0b, labelled as one, since
the clause names only `.git` and `vendor/foo.git/` carries none), and its `Unreadable` state stops
the walk rather than being read as "nothing here". `tests/boundary_walks.rs` tests the **class** —
one tree holding all three shapes, every walker asked whether it crossed — so a future walker that
rolls its own skip is caught there rather than in somebody's repository.

That removes the blocker this paragraph named: 0b is now implementable as a per-candidate predicate
on top of the shared classifier.

---

## 3. §9.3 is wrong, or incomplete, in every conjunct

Each item below was verified by running the tool, not by reading about it. This is the corpus the
implementation should be built from — and the second time today the research document has proved
excellent on mechanism and unreliable on specifics.


### Gate 0a

- **§6.13's DVC premise is not DVC's default configuration.** Official DVC docs, fetched and verified this session: *"By default, DVC tries to use reflinks for the cache if available on your system, however this is not the most common case at this time, so it falls back to the copying strategy."* Symlinks require an explicit `dvc config cache.type ... symlink`. So §6.13's *"the workspace will only contain links to the data files in the cache"* describes a configured setup, not the default: in a default DVC repo 0a sees no links at all and the DVC half of the exception never engages, and on macOS/APFS (where reflinks are supported) the loss channel is §6.19's reflink case, which 0a cannot see. Implement the DVC gate as a proxy and say so in the module doc; do not present it as a test for "DVC data is at risk".
- **git-annex content is not always a symlink.** Official git-annex internals page, fetched and verified this session: *"Files added to the annex get a symlink **or pointer file** checked into git, that points to the file content."* Unlocked / adjusted-branch annex repos store pointer *files* — regular files — which 0a's link rules cannot see. Only the repo-level probe and Gate 1's content rules catch those. Record as a gap; do not let the module doc imply that annexed content is always visible to 0a.
- **`.dvc/cache` is DVC's default location, not its only one.** DVC supports a configurable `cache.dir` (and a site cache dir), so a prefix match on `.dvc/cache` in `DataStore::named_by_target` is incomplete. Document it as a **sufficient** test for `ContentPointer` and never a necessary one — which is exactly why the per-link rule must not be the only defence, and why `StorePresence::Unreadable` must behave as `Present`.
- **`.git/annex/objects` is confirmed exactly as §6.13 states** (git-annex internals: `.git/annex/objects/aa/bb/*/*`). No divergence — recorded because contract 4 asks for the verification either way.
- **§9.3 says "lstat everything" without saying that lstat is only meaningful on a separator-free spelling.** Measured: `symlink_metadata("link/")` returns the *target's* metadata with `is_symlink=false`; `symlink_metadata("filelink/")` gives ENOTDIR 20; `symlink_metadata("dangling/")` gives ENOENT 2. Implementation must strip trailing separators lexically before any syscall. This is not in the spec and is the largest single correction to the round-1 design.
- **§6.16's "never rm -rf a target" is verified and is stronger than the spec states.** Measured on this machine: `/bin/rm -rf LINK/` deleted the target directory *and* its contents and left the link dangling, while `/bin/rm -rf LINK` removed only the link; `find LINK/ -type f` enumerated the target while `find LINK -type f` returned nothing; and **`std::fs::remove_dir_all("LINK/")` destroys the target's contents too** while `remove_dir_all("LINK")` removes only the link. The hazard is in std, not only in the shell, so it cannot be dismissed as a shell-invocation concern.
- **Gate 1's own store probe is factually wrong for linked worktrees and submodules.** `state.rs:868` probes `<root>/.git/annex`. Verified: in a linked worktree and in a submodule, `<root>/.git` is a **file**, and `git rev-parse --git-common-dir` returns the main repo's `.git` and `.git/modules/<name>` respectively. 0a must use the common dir; the shared `store.rs` fixes both gates, and `state.rs:868`'s use of `.exists()` (follows links, returns `false` on any error) is fixed with it.
- **§9.3 says what to *report*, never what may be *done to the link itself*.** 0a refuses a resolving link as a candidate and Clears a store-free dangling one; it does not say whether the dangling pointer may be unlinked. Needs a determination in `docs/decisions/` before any auto-act, in the same form as the existing `2026-08-02-r1-determination.md`.
- **Contract 4 also names §9.3 0f's `target/.cargo-lock`. Independently re-verified here: `target/debug/.cargo-lock` exists in this repo, `target/.cargo-lock` does not.** Out of 0a's scope; recorded so the divergence is not attributed to 0a and so 0a does not inherit 0f's literal `*.lock` reading.

### Gate 0b

- THE ROOT IS EXEMPT, WHICH THE CLAUSE DOES NOT SAY. §9.3 0b reads 'any directory containing .git'; the working tree root contains `.git` by definition, so the clause read literally is a constant function that refuses every repository. Implemented as: the walk starts AT the root and probes only components below it, so the exemption is structural and there is no `.` / empty-path special case to get wrong. The exemption is on BEING the root, not on what the root's marker looks like — verified a linked worktree's root carries a gitfile (`gitdir: <main>/.git/worktrees/wt`), so a 'root has a `.git` directory' phrasing would refuse every worktree-per-task tree.
- 'CONTAINING .git' DOES NOT DISTINGUISH DIRECTORY FROM FILE, AND A DIR-ONLY TEST FAILS OPEN ON BOTH CASES THE CLAUSE NAMES. Verified: `git submodule add` writes `sub/.git` as a regular FILE containing `gitdir: ../.git/modules/sub`, and `git worktree add wt` writes `wt/.git` as a regular FILE containing `gitdir: <abs>/outer/.git/worktrees/wt`. Implemented across four marker types — directory, gitfile, symlink, and foreign file type (FIFO/socket/device, verified reachable via `lstat` reporting all of is_dir/is_file/is_symlink false).
- THE CLAUSE IS PURELY FILESYSTEM AND MISSES THE DEFAULT STATE OF EVERY SUBMODULE AFTER A PLAIN CLONE. Verified: cloning a superproject without `--recurse-submodules` leaves `sub/` as an EMPTY directory with no `.git` marker, while the index still carries `160000 … sub`. The index gitlink is the only evidence, so 0b reads `git ls-files --stage -z` as well as the filesystem. This is not an optional completeness pass: verified `git ls-files --error-unmatch -- ':(literal)libs/foo'` on a mode-160000 entry exits 0, so `Repo::recoverability()` (git.rs:507) returns TrackedPushed — the only class §8.1 L966 admits to auto-action — for a directory whose content is in `.git/modules/<name>` and nowhere in this object database.
- BARE REPOSITORIES HAVE NO `.git` AT ALL AND ARE MISSED ENTIRELY BY THE CLAUSE (the Gate 0 extraction already flags them as 'unaddressed'). Implemented as a WIDENING, labelled as one in the module doc, using git's own `is_git_directory()` conjunction: `HEAD` a regular file, `objects/` a directory, `refs/` a directory, and `HEAD`'s first 64 bytes beginning `ref: ` or being a 40-hex oid. Verified against `git init --bare` (`HEAD` = `ref: refs/heads/main`) and against `git init --bare --ref-format=reftable` (`HEAD` = `ref: refs/heads/.invalid`, with `objects/` and `refs/` both still present). The conjunction is what makes accidental firing essentially impossible; on this repository it short-circuits on the first conjunct in all 28 directories.
- THE CLAUSE GOVERNS DESCENT AND IS SILENT ON THE MARKER-CARRYING DIRECTORY AS A CANDIDATE. Implemented as a WIDENING: 0b refuses the container too. Permitting `rm -rf vendor/tool` while forbidding `rm -rf vendor/tool/src` is incoherent, deleting the container is precisely §8.3 L977's hazard, and for a declared submodule the container is the thing 0g classifies TRACKED_PUSHED. Labelled as a widening in the module doc; the parent owns the call and inverting it is one branch.
- 'REFUSE TO DESCEND' IS AN INSTRUCTION TO A WALKER, BUT NO SHARED GATE-0 WALK EXISTS. Verified: `gate3f::walk` (gate3f.rs:659) and `StateGate::survey` (state.rs:857) each have their own loop and each merely skips a directory literally NAMED `.git` (gate3f.rs:673, state.rs:989), so both will keep descending into a nested linked worktree and a bare `vendor/foo.git/` after 0b lands. 0b is therefore implemented as a per-candidate predicate consumed by `judged explain` and the E2 layer, with the walker retrofit recorded in the module doc as an open gap rather than described as done. Additional finding while verifying this: `gate3f::walk` uses `path.is_dir()`, which follows symlinks, so that walker traverses symlinked directories today — a 0a violation the same retrofit should close.
- §9.3's Gate 0 header calls 0a– 0g 'structural refusals' but only 0d and 0f say 'auto-act', leaving report-only mode unstated. Implemented as: 0b binds ALWAYS. Verified that report-only is where the harm lands — inside an embedded clone the outer repo reports UNTRACKED ('zero recovery path') for a committed and pushed file, and for a declared submodule directory it reports TRACKED_PUSHED ('safe to auto-act'). Both sentences are printed by `judged explain` today, so binding in report-only is what makes the report true rather than a policy preference.
- NO CONCRETE PATH IN 0b IS WRONG, WHICH IS WORTH SAYING GIVEN 0f. 0b names exactly one path token, `.git`, and I verified it is correct as a NAME and incomplete as a TEST (see the divergences above). The three further paths this design relies on were each verified against the tool rather than taken from the spec: the gitfile prefix is exactly `gitdir: ` (both worktree and submodule forms), the bare triple is `HEAD`/`objects`/`refs` under both ref backends, and the gitlink mode string is `160000` in `ls-files --stage -z` output whose records are `<mode> SP <oid> SP <stage> TAB <path>` with paths emitted RAW (confirmed by `od -c`).

### Gate 0c

- §9.3 L1039 says 'Canonicalize paths; reject any candidate whose realpath is not a repo descendant.' The implementation does NOT call realpath(3)/fs::canonicalize on any candidate. Verified reasons: realpath returns ENOENT for any path that does not yet exist, so every already-deleted or to-be-created candidate becomes unanswerable and the only remaining move is the lexical fallback both existing copies already take (git.rs:427, contracts.rs:1531); realpath dereferences the final component, so its answer names the symlink's target, which is precisely what 0a forbids handing to a caller; and it returns a single path, so it cannot report which component escaped, which §6.16 requires. The same containment relation is computed with lstat(2) + readlink(2) in an explicit component walk. Record this in the module doc as the first gap.
- Round 1's own refusal case #4 asserted the npm-link escape looks like `node_modules/mylib -> /Users/x/dev/mylib`, an absolute target. VERIFIED FALSE on npm 11.9.0 / node v24.14.0: `npm link` in the consumer writes a RELATIVE link, `node_modules/judged-fx-mylib -> ../../mylib`. Implementation consequence: there must be no 'absolute target = suspicious' heuristic; only resolving the relative target against the resolved parent catches it. Verified the revised walk does: Refuses(EscapedTarget, lands=<scratch>/npmtest/mylib).
- §8.5 L989-993's `~/.local/state/<tool>/<repo>/<ISO-date>/` is NOT unconditionally outside the repository, so the implementation must not treat it as a constant-safe default destination. Verified with a $HOME-shaped root: the sanctioned path resolves strictly inside and judge_quarantine_destination refuses it. §8.5 itself supplies the fallback — a ref/tag (R2) or a bundle (R1) — and the quarantine wiring must consult the probe rather than assume.
- §6.16 L675's claim that bazel-out/bazel-bin/bazel-testlogs are symlinks into ~/.cache/bazel could NOT be re-verified on this machine: bazel is not installed (`which bazel` -> not found). It is carried as spec-sourced. This costs the design nothing, because 0c hardcodes no path names at all — it is a structural check, so whether the directory is called bazel-out, bazel-bin, or something a future release renames it to does not change a line of code. The shape was verified with a hand-built fixture instead, live and dangling.
- Ordering divergence the spec does not state: 0c must run BEFORE any candidate path reaches git, not after. Verified: git's pathspec normalization is lexical and disagrees with the kernel across a symlink. `git ls-files --error-unmatch -- ':(literal)esc/../README.md'` prints README.md and exits 0 — i.e. reports 'tracked, therefore recoverable' — for a path that `ls` reports as No such file or directory and whose containing directory is outside the repository. `git check-ignore -vz --stdin` behaves the same way. Gate 0g's recoverability answer is therefore only trustworthy for a path 0c has already cleared.

### Gate 0d

- `.git/shallow` (L1042) is not a portable path and must not be tested literally. Verified: in a linked worktree of a `--depth 1` clone, `--is-shallow-repository` is true while `test -e .git/shallow` is false, because `.git` is a gitfile and `--git-path shallow` resolves to `<main>/.git/shallow`; in a submodule it resolves to `<super>/.git/modules/<name>/shallow`. A literal implementation of the spec's words fails open in exactly the two configurations 0b and 0d both care about. Implement `git rev-parse --is-shallow-repository`.
- `.git/shallow present` must NOT be read as `Repo::is_shallow()`. That predicate also returns true for any promisor clone, and verified worse: a `--filter=blob:none` clone whose server ignored the filter (`warning: filtering not recognized by server, ignoring`) still has `remote.origin.promisor=true`, so a complete clone with every object present is classified shallow. 0d implements the narrower `ShallowState::Grafted`; the fused predicate stays where it belongs, in the evidence-abstention path (`veto/recency.rs:288`).
- `HEAD not on any remote` (L1042) via Gate 0g's definition is not sufficient to license deletion. `git.rs:394` already predicts it — the local `refs/remotes/**` cache can claim a commit is published after a force-push removed it — and it reproduces exactly (cache says `refs/remotes/origin/main` contains HEAD while the server's main is an unrelated orphan). Gate 0g may keep the cache-only definition, since a wrong answer there mislabels a rung; 0d must confirm against the remote, since there the same wrong answer *is* the licence to act.
- `rebase/merge/bisect/cherry-pick in progress` (L1040) names four operations but requires seven `$GIT_DIR` paths, and the honest set includes two the spec does not name: `REVERT_HEAD` (revert is cherry-pick's sequencer twin and drives the identical half-applied state) and `rebase-apply/` which is also where `git am` stops. Both are labelled widenings in the table's `clause` column rather than only in prose.
- `running inside a worktree a parent may force-remove` (L1042) has no detection procedure anywhere in the document, and the obvious implementation misses two of the three shapes: verified that in a submodule `--git-dir == --git-common-dir` (so the worktree test is silent) and that in an independent nested clone both the worktree test and `--show-superproject-working-tree` are silent while `git clean -ffdx` in the outer repo removes it.
- `git worktree list --porcelain` has no current-worktree marker in any form — verified from both a main and a linked worktree. Any design that reads one is reading a field that does not exist; identity has to be derived by canonical path equality against `Repo::root()`, and the derivation has a real failure mode (a hand-moved worktree lists its stale path), which must surface as `Unreadable`, not as a silent non-match.

### Gate 0e

- `.git/lfs/objects` is the DEFAULT, not the location. Verified from git-lfs-config(5) upstream: `lfs.storage` — "Allow override LFS storage directory. Non-absolute path is relativized to inside of Git repository directory (usually .git). Default: `lfs` in Git repository directory (usually .git/lfs)." An absolute `lfs.storage` puts the object store anywhere on the filesystem, outside `.git` entirely — and §6.13 says that content "may exist on no remote". A `.git/`-anchored region set misses it completely. IMPLEMENT: read `git config --get lfs.storage` at build (verified: exit 1 when unset is an answer, exit 0 with the value) and add it as `Boundary::LfsStorageOverride`. Also verified that `.lfsconfig` CANNOT set it — git-lfs restricts that file to `gitprotocol, locksverify, pushurl, skipdownloaderrors, url, *.access, remote.*.lfsurl` "for security reasons" — so `git config` is authoritative and no file parsing is needed.
- Do NOT derive the named sub-regions with `git rev-parse --git-path`. Verified in a linked worktree on 2.50.1: `git rev-parse --git-path lfs/objects` returns `…/.git/worktrees/wt1/lfs/objects`, where neither git-lfs nor git-annex stores anything, while `--git-path objects` correctly returns the common dir's. git's per-worktree/common path table knows about `objects` and knows nothing about `lfs` or `annex`. IMPLEMENT: derive sub-regions by joining, and join against BOTH `$GIT_DIR` and `$GIT_COMMON_DIR`, because git-lfs's own documentation says "usually .git" without saying which of the two it means in a worktree, and git-lfs and git-annex are not installed on this machine so the question could not be settled empirically.
- "Never touch `.git/`" as a path string is factually wrong on three shapes, all verified. (a) `--separate-git-dir`: `--absolute-git-dir` returns `/…/elsewhere` — no `.git` component anywhere in the path. (b) Submodule checkout: `sub/.git` is a 28-byte regular FILE containing `gitdir: ../.git/modules/sub`, and `$GIT_DIR` is `…/.git/modules/sub`. (c) Linked worktree: `$GIT_DIR` is `…/.git/worktrees/wt1` while `$GIT_COMMON_DIR` is `…/.git`, so a single "the git dir" is not enough. IMPLEMENT: the plumbing-resolved set, plus `<root>/.git` as it exists on disk (which `git rev-parse` never names).
- 0e's letter is contradicted by the tool's own mandated writes. §8.2's `git add -f`, `GIT_INDEX_FILE=/tmp/idx … git write-tree`, `commit-tree`, `git tag`, and §9.7's quarantine refs all write into `.git` and are REQUIRED. IMPLEMENT the intended reading — never as a filesystem mutation target — and route none of those through `judge`. Recorded in the module doc as a gap, not hidden. §8.3's `gc.auto` self-sabotage (6,700 loose objects triggering the reflog expiry the quarantine depends on) constrains those writes and has no gate clause anywhere in §9.3; it belongs to whoever implements the promotion, not to 0e.
- CORRECTION TO ROUND 1'S OWN VERIFICATION, which the refutation did not catch. Round 1 claimed "verified on APFS that the path exists and `realpath` returns `.GIT` without folding case", and built the argument that the `(dev, ino)` matcher is the only authoritative check on that claim. It is false. Verified by calling libc `realpath(3)` directly and by `std::fs::canonicalize`: on APFS, `.GIT/config` canonicalizes to `.git/config` and `FOO/Bar/BAZ.txt` to `Foo/BAR/Baz.TXT` — macOS `realpath(3)` DOES fold every component to its on-disk spelling. Round 1's contrary result came from Python's `os.path.realpath`, which is a lexical implementation that never asks the filesystem for the canonical name. CONSEQUENCE: the case hazard is handled by resolution itself on this platform; the ASCII-case-insensitive component test is retained only for the lexical fallback on non-existent paths and for platforms whose `realpath` may not fold. It is a supplement, not the load-bearing check, and the module doc must not repeat round 1's claim.
- Use `git rev-parse --absolute-git-dir --git-common-dir`, not `--path-format=absolute`. Verified both produce identical results on 2.50.1. `--absolute-git-dir` exists since git 2.13 (2017); `--path-format` needs 2.31 (2021), which would silently exclude e.g. Ubuntu 20.04's git 2.25. `--git-common-dir` is the only line that can come back relative (verified: prints `.git` in a main repo) and is joined against `repo.root()`, which `Repo::discover` already canonicalized. One code path, no fallback branch to keep honest, and round 1's open question on the git version floor is closed.

### Gate 0f

- **`target/.cargo-lock` does not exist and never has.** Cargo writes `<target-dir>[/<triple>]/<profile>/.cargo-lock`. Verified twice on cargo 1.94.1: `find target -name .cargo-lock` in this repo returns `target/debug/.cargo-lock`; a scratch `cargo build --target aarch64-apple-darwin --target-dir tdir2` produced both `tdir2/debug/.cargo-lock` and `tdir2/aarch64-apple-darwin/debug/.cargo-lock`. Implementation matches the **file name** `.cargo-lock` anywhere in the tree, which covers every profile, every target triple, and any in-tree `CARGO_TARGET_DIR`. A separate rule is required because `.cargo-lock` ends `-lock` and is therefore not matched by the spec's `*.lock` glob.
- **`node_modules/.package-lock.json` is not a 'build is running' signal and is removed from the marker set.** Measured: over a 10 s `npm install` polled at 1.5 s, its mtime stayed at the *previous* install's value for 9 s and was updated one second before completion. Firing on it would clear during the install and refuse for the window after it — inverted. The obvious substitute, `node_modules`' own directory mtime, was measured too and also fails (one bump at t≈0, static for 11 s). npm, pnpm and yarn take no advisory lock; 0f cannot see them and says so in `Coverage::structural` and in `Sut::cannot_emit`.
- **`.next/trace` is removed from the marker set.** Next.js takes no advisory lock, so there is no heldness path, and I could not run a Next.js build on this machine to measure whether `trace` is written continuously during the activity it would need to detect. Having just measured that the *other* mtime marker was inverted, shipping an unmeasured mtime rule would be a guess. Recorded as a gap rather than implemented; reinstatable if someone measures it.
- **`*.lock` is read as heldness, never as presence.** §9.3 never distinguishes lock-as-mutex from lockfile-as-manifest. Read as presence it refuses `Cargo.lock`, `poetry.lock`, `flake.lock`, `uv.lock`, `Gemfile.lock`, `composer.lock`, `bun.lock`, `deno.lock` — measured: 155 such files in this repo, 9 in a plain clone of `BurntSushi/memchr`, all free, all permanent. The implementation refuses only on an observed exclusive holder.
- **The whole freshness/mtime mechanism §9.3 implies is absent.** 0f refuses on OS-observable heldness only. This makes the gate narrower than the spec's sentence: it detects cargo and rustc (measured) and any flock-based tool, and detects nothing else. Per `gate3f.rs:35-62`'s own precedent — *'being narrower than a safety rule is the wrong direction to err in, so it is recorded as a gap rather than described as the rule'* — this is written into the module doc as a gap. It means 0f does **not** discharge §11 R11's open-FD half; R11's *'before any auto-act ships'* is not satisfied by 0f alone.
- **The advisory lock is written inside `.git`** (`<git-common-dir>/judged-run.lock`), which requires 0e's *'Never touch .git/'* to be read as a never-**delete** set. That is plainly the intended reading — §8.2's `git add -f` / `commit-tree` / `git tag` and §9.7's quarantine refs all write there — but 0e does not say so, and this design depends on it. §9.3 does not say where the lock lives at all; the choice, its scope (per clone, shared across linked worktrees) and its mode (shared for report-only, exclusive for auto-act) are determinations this design makes, not spec text.
- **§9.3's 'Refuse' is read as 'refuse to mutate', not 'abort the run'.** L1044 says 'Refuse' where L1040 (0d) says 'Refuse to auto-act'; the gate header calls all of 0a-0g 'structural refusals'. This design rules that report-only continues and prints 0f's refusals as a header, because a report-only run writes no ledger and no quarantine, so R11's stated justification does not reach it. Recorded as a determination for consistency with 0a-0e.

---

## 4. Why no code

Two rounds, twenty-six agents, zero sound designs. The remaining problems are no longer "this design
commits the cardinal sin" — they are constructibility and integration defects that need the whole
crate in view: a design whose `Candidate` type structurally cannot carry the field its own answer
depends on; a design returning `Vec<Conflict>` where `Conflict` exists nowhere in the codebase,
while ignoring the `Outcome`/`GateState` carrier that does; a walk order that never reaches its own
load-bearing branch, reproduced end-to-end against a cone-mode sparse checkout.

Those are fixable. They are not fixable well by writing six modules of subtle filesystem and git
code at the end of a long session, which is precisely how the three defects external review caught
today got in. The contract in §1 and the corpus in §3 are what make the next attempt cheap; writing
the code now would spend them.

**Start here:** 0e and 0a first — the handoff's reasoning still holds, they are refusals with no
measurement behind them and their absence is the difference between a bug and an unrecoverable one.
But note that §3 shows 0e's "never touch `.git/`" is wrong as a *path string* on three verified
layouts, so it must be implemented as an identity test against `--absolute-git-dir` and
`--git-common-dir`, never as a prefix match. Fix `state.rs:868` first; it is a two-line change and
it is already wrong in the tree.
