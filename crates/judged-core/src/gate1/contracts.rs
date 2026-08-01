//! Gate 1 classes 1l–1p — contracts with the outside world, and the tool
//! protecting itself from itself.
//!
//! Five classes, and the first four share one shape: a file that is **tracked,
//! referenced by nothing in the repository, and whose deletion fails silently**.
//! Every reachability analyzer, every call graph, every module resolver returns
//! UNUSED on all of them with maximum confidence, and every one of those answers
//! is *correct* — the edge that keeps the file alive simply does not live in
//! this repository. That is why these classes sit in Gate 1 and not in a
//! confidence tier: no amount of additional in-repo evidence moves them.
//!
//! - **1l — platform contracts** (§6.11). Read by GitHub Pages, Netlify, Apple,
//!   Google, a certificate authority, an ad exchange, the forge, the hosting
//!   platform. See [`platform_contracts`].
//! - **1m — un-ignored by a `!` negation** (§6.17). Files whose entire purpose
//!   is to exist, carved back out of an ignored tree. See [`NegationUnIgnore`].
//! - **1n — the keep manifest and the deletion ledger** (§6.22).
//! - **1o — the tool's own evidence artifacts** (§6.22).
//! - **1p — the unknown defaults to KEEP.** The fallback, in code.
//!
//! # 1l: implement the predicate, not the proxy
//!
//! §6.11 raises and then rejects the obvious implementation: *"a proposed rule
//! 'hard-exclude files under ~64 bytes' correctly saves `__init__.py`,
//! `.gitkeep`, `py.typed`, `.nojekyll` — and `CNAME` at ~20 bytes is exactly
//! that class but the floor is a heuristic about size when the real predicate is
//! **read by something outside the repository**."*
//!
//! So the predicate is encoded, not approximated. [`PlatformContract`] has no
//! size field, [`PlatformContract::matches`] never opens the file, and every row
//! of [`PLATFORM_CONTRACTS`] must name the [`PlatformContract::consumer`] that
//! reads it and the [`PlatformContract::effect`] deleting it causes. An entry
//! that cannot name an external reader cannot be written down. The consequence
//! is the one a size floor cannot produce: a 200 KB
//! `apple-app-site-association` refuses and a 6-byte `src/util.py` does not.
//!
//! The [`FailureMode`] on each row exists because these failures are not 404s
//! and a report that says only "keep this" throws away the reason anyone would
//! agree. Deleting `CNAME` removes the GitHub-side binding while DNS still
//! points at GitHub — the dangling-DNS condition behind **subdomain takeover**,
//! a security event. Deleting `.well-known/acme-challenge/*` stops automated TLS
//! renewal, surfacing weeks later as an expired certificate. Deleting `ads.txt`
//! makes ad inventory unauthorized and revenue goes to zero with no error
//! anywhere. Deleting `CODEOWNERS` removes required-review enforcement, and the
//! failure is that CI stops failing. Deleting `apple-app-site-association`
//! breaks Universal Links for apps already on other people's phones.
//!
//! # 1m: ask git, and ask it about the file
//!
//! §6.17 measured 246 negation patterns across 41 of the 312 canonical
//! `github/gitignore` templates, re-including `.vscode/settings.json`,
//! `var/logs/.gitkeep` and `/media/**/.htaccess` — *"files whose entire purpose
//! is to exist"*. Its experimental result is that **git itself is per-file
//! careful and every naive `rm -rf`-on-ignored-directories reimplementation is
//! not**, so this module reimplements nothing: it runs `git check-ignore` and
//! reads the deciding rule. Precedence between nested `.gitignore` files,
//! `.git/info/exclude`, `core.excludesFile`, and the rule that a negation cannot
//! re-include a file under an excluded *directory* are all git's to know.
//!
//! Two details are load-bearing and both were measured against git 2.50.1:
//!
//! - **`--no-index` is required.** Without it, `git check-ignore -vz --stdin
//!   --non-matching` reports an empty pattern for every *tracked* path — it
//!   consults the index, answers "not ignored", and never says which rule
//!   decided. Every file §6.17 measured is checked in, so a probe without
//!   `--no-index` sees exactly none of them.
//! - **The pattern, not the exit code.** "Not ignored" covers two different
//!   worlds: no pattern matched at all, and a pattern matched and re-included
//!   the file. Only the second is class 1m. [`crate::git::Repo`] asks the
//!   coarser question (is this ignored, for recoverability) and collapses both
//!   into `false`; this module needs the finer one, so it reads the reported
//!   pattern and keeps the `.gitignore` file and line as evidence.
//!
//! # 1n and 1o: the tool is inside its own blast radius
//!
//! §6.22 is not paranoia, it is a description of a loop that closes:
//!
//! - The keep manifest *"accumulates entries and looks stale; an agent told to
//!   'clean up the repo' prunes the veto list, and the **next** run deletes
//!   everything the human previously vetoed."* The prescription is two-part —
//!   the manifest is the first entry in its own never-touch list, and *"pruning
//!   it must be structurally impossible in the same run as any deletion"*. The
//!   first part is [`TOOL_ARTIFACTS`]; the second is [`review_plan`].
//! - `.coverage`, `lcov.info`, `jacoco.exec`, `*.profraw` and the analysis
//!   caches are *simultaneously canonical junk patterns and the tool's
//!   evidence*. A cleaner that removes them has strictly less evidence next run,
//!   **and nothing detects that the cause of the missing data was the cleaner
//!   itself**, so confidence degrades monotonically toward more aggressive
//!   deletion with each run. Every 1o row carries
//!   [`ToolArtifact::also_canonical_junk`] so the collision is stated rather
//!   than discovered.
//!
//! Two limits, stated because a limit a caller cannot see is indistinguishable
//! from a bug. §6.22 writes "and every build cache"; this module does not
//! implement that literally, because `target/`, `dist/` and `node_modules/` are
//! the tool's entire reason to exist and a Gate 1 class that refuses them
//! refuses everything. 1o covers the measurements and report artifacts whose
//! loss silently changes what the *next* run can conclude, plus the two caches
//! §6.22 names (`.nyc_output/`, `.turbo/`); the general "do not delete what this
//! run read evidence from" rule is a run-level invariant, not a path list. And
//! [`review_plan`] takes the plan as given: it cannot know about a deletion its
//! caller never declared.
//!
//! # 1p: the unknown defaults to KEEP
//!
//! A file whose type cannot be determined is not a candidate — and this is the
//! *fallback*, so it is a branch in [`ContractGate::classify`] rather than a
//! sentence in a doc comment. A type is determined by a recognised extension, by
//! recognised leading bytes, or by a name the ecosystem gives a fixed meaning
//! ([`TypeSignal`]). Nothing else counts: in particular a file being valid UTF-8
//! is **not** a determination, because knowing a file is text tells you nothing
//! about what reads it.
//!
//! 1p stacks rather than overrides: a file re-included by a `!` negation whose
//! type nothing can determine reports 1m *and* 1p, because "somebody wrote a
//! rule to keep this" and "we do not know what it is" are different arguments
//! and a human needs both.
//!
//! It does not stack with 1l, and that is deliberate. Being a platform contract
//! **is** knowing what a file is — `CNAME`, `Procfile` and
//! `.well-known/change-password` carry no extension and no magic bytes, and a
//! verdict reading "GitHub Pages reads this" *and* "we cannot determine what
//! this is" contradicts itself. Location is the type here; it is precisely what
//! the external reader keys on. The safety argument for entangling the two, and
//! the reason it is allowable: consulting the contract registry inside type
//! determination can never flip a verdict toward deletion, because every path it
//! recognises has already pushed a 1l refusal. It can only drop a redundant
//! reason from a file that is refused either way.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::git::Repo;
use crate::{Error, Result};

// ---------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------

/// Which of §9.3's classes 1l–1p refused.
///
/// Ordered as the classes are lettered, so a verdict's class list reads in a
/// stable order no matter which check ran first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContractClass {
    /// 1l — read by something outside the repository.
    PlatformContract,
    /// 1m — un-ignored by a `!` negation.
    NegationUnIgnored,
    /// 1n — the keep manifest and the deletion ledger themselves.
    ToolLedger,
    /// 1o — the tool's own evidence artifacts.
    ToolEvidence,
    /// 1p — the type could not be determined.
    UnknownType,
}

impl ContractClass {
    /// The §9.3 class letter, for reports a human reads next to the document.
    pub fn code(self) -> &'static str {
        match self {
            ContractClass::PlatformContract => "1l",
            ContractClass::NegationUnIgnored => "1m",
            ContractClass::ToolLedger => "1n",
            ContractClass::ToolEvidence => "1o",
            ContractClass::UnknownType => "1p",
        }
    }
}

impl std::fmt::Display for ContractClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// What deleting a platform-contract file actually does.
///
/// Every variant is a *silent* failure — none of them is a 404, and none of them
/// surfaces at the moment of deletion. The variant is carried so a report can
/// say why, and so a caller can sort by consequence rather than by filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureMode {
    /// The platform-side binding disappears while DNS still points at the
    /// platform — the dangling-DNS condition that enables subdomain takeover.
    SecurityEvent,
    /// A control that gates merges, updates, or transport stops being enforced.
    /// The failure is that nothing fails.
    ControlSilentlyRemoved,
    /// Delivery, verification, or revenue stops with no error emitted anywhere.
    DeliveryStopsSilently,
    /// Clients already installed on other people's devices break, and stay
    /// broken until caches expire.
    ShippedClientsBreak,
    /// Automated renewal stops. It surfaces weeks later, as an expiry.
    RenewalStopsUntilExpiry,
    /// Routing, headers, or build behaviour read only by the platform changes.
    PlatformBehaviourChanges,
}

/// How a registry row recognises a path. Deliberately three shapes and no
/// glob engine: every §6.11 row is a fixed name, a fixed path, or a fixed
/// directory, and a pattern language here would be a second place for a bug to
/// live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Matcher {
    /// The final path component equals this, at any depth. Case-sensitive,
    /// because the external reader is: GitHub Pages reads `CNAME`, not `cname`.
    Basename(&'static str),
    /// The whole repository-relative path equals this.
    ExactPath(&'static str),
    /// These segments occur consecutively somewhere in the path, and the path
    /// continues past them — i.e. the file is *inside* that directory. Written
    /// this way rather than as a root-anchored prefix because `.well-known/`
    /// lives under `public/`, `static/`, `web/` and the repository root
    /// depending on the framework.
    Inside(&'static str),
    /// The file extension equals this, without its dot.
    Extension(&'static str),
}

impl Matcher {
    fn matches(&self, rel: &Path) -> bool {
        match self {
            Matcher::Basename(name) => rel.file_name().map(|n| n == *name).unwrap_or(false),
            Matcher::ExactPath(path) => rel == Path::new(path),
            Matcher::Inside(dir) => {
                let needle: Vec<&str> = dir.split('/').collect();
                let parts: Vec<&str> = match rel.iter().map(|c| c.to_str()).collect() {
                    Some(parts) => parts,
                    // A non-UTF-8 component cannot equal an ASCII segment name.
                    None => return false,
                };
                // `windows` is empty when the path is shorter than the needle,
                // and the file must be strictly inside, so the needle can never
                // occupy the last component.
                if parts.len() <= needle.len() {
                    return false;
                }
                parts[..parts.len() - 1]
                    .windows(needle.len())
                    .any(|w| w == needle.as_slice())
            }
            Matcher::Extension(ext) => rel.extension().map(|e| e == *ext).unwrap_or(false),
        }
    }

    /// A stable human-readable form, used in reports and as the registry's
    /// uniqueness key.
    fn describe(&self) -> &'static str {
        match self {
            Matcher::Basename(name) => name,
            Matcher::ExactPath(path) => path,
            Matcher::Inside(dir) => dir,
            Matcher::Extension(ext) => ext,
        }
    }
}

// ---------------------------------------------------------------------------
// 1l — platform contracts (§6.11)
// ---------------------------------------------------------------------------

/// One row of §6.11: a file read by something that is not this repository.
///
/// The struct is the predicate. There is no size, no age, no reference count and
/// no confidence — only the identity of the external reader and what happens to
/// it when the file stops existing.
#[derive(Debug)]
pub struct PlatformContract {
    matcher: Matcher,
    consumer: &'static str,
    effect: &'static str,
    failure: FailureMode,
}

impl PlatformContract {
    /// The thing outside this repository that reads the file.
    pub fn consumer(&self) -> &'static str {
        self.consumer
    }

    /// What deleting the file does, in one clause.
    pub fn effect(&self) -> &'static str {
        self.effect
    }

    /// How the failure presents.
    pub fn failure_mode(&self) -> FailureMode {
        self.failure
    }

    /// The name, path, directory or extension this row recognises.
    pub fn pattern(&self) -> &'static str {
        self.matcher.describe()
    }

    fn matches(&self, rel: &Path) -> bool {
        self.matcher.matches(rel)
    }
}

/// §6.11's table.
///
/// Order is precedence: the first row that matches wins, so the specific
/// `.well-known/` contracts precede the `.well-known/` catch-all. Every row
/// names its consumer, because that — not size, not age, not reference count —
/// is the predicate.
static PLATFORM_CONTRACTS: &[PlatformContract] = &[
    // -- GitHub Pages and the forge ------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("CNAME"),
        consumer: "GitHub Pages (custom-domain binding)",
        effect: "removes the GitHub-side domain binding while DNS still points at GitHub",
        failure: FailureMode::SecurityEvent,
    },
    PlatformContract {
        matcher: Matcher::Basename(".nojekyll"),
        consumer: "GitHub Pages (build pipeline)",
        effect: "Pages runs Jekyll and silently 404s every _next/, _app/ and _astro/ path",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("_config.yml"),
        consumer: "Jekyll on GitHub Pages",
        effect: "site configuration, baseurl and plugin set revert to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("CODEOWNERS"),
        consumer: "the forge's required-review enforcement",
        effect: "silently removes required-review enforcement — a security control",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::ExactPath(".github/dependabot.yml"),
        consumer: "GitHub Dependabot",
        effect: "silently stops security updates",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::ExactPath(".github/dependabot.yaml"),
        consumer: "GitHub Dependabot",
        effect: "silently stops security updates",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename("renovate.json"),
        consumer: "Renovate",
        effect: "silently stops dependency and security updates",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename(".renovaterc.json"),
        consumer: "Renovate",
        effect: "silently stops dependency and security updates",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Inside(".github/workflows"),
        consumer: "the forge's CI scheduler",
        effect: "a scheduled job — a nightly backup, a certificate refresh — silently stops running",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::ExactPath(".github/FUNDING.yml"),
        consumer: "the forge's sponsor UI",
        effect: "the sponsor button disappears; consumed by the forge, never by the repository",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Inside(".github/ISSUE_TEMPLATE"),
        consumer: "the forge's issue composer",
        effect: "issue templates and their routing disappear",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("action.yml"),
        consumer: "other repositories, via `uses: org/repo/path@ref`",
        effect: "breaks every workflow in every OTHER repository that composes this action",
        failure: FailureMode::ShippedClientsBreak,
    },
    PlatformContract {
        matcher: Matcher::Basename("action.yaml"),
        consumer: "other repositories, via `uses: org/repo/path@ref`",
        effect: "breaks every workflow in every OTHER repository that composes this action",
        failure: FailureMode::ShippedClientsBreak,
    },
    // -- Netlify, Vercel, Cloudflare Pages -----------------------------------
    PlatformContract {
        matcher: Matcher::Basename("_redirects"),
        consumer: "Netlify / Cloudflare Pages edge router",
        effect: "every rewrite and redirect stops; SPA deep links start 404ing",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("_headers"),
        consumer: "Netlify / Cloudflare Pages edge router",
        effect: "silently removes CSP and HSTS",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename("netlify.toml"),
        consumer: "Netlify build and routing",
        effect: "build command, publish directory, redirects and headers revert to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("vercel.json"),
        consumer: "Vercel build and routing",
        effect: "rewrites, headers, regions and function config revert to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("static.json"),
        consumer: "the Heroku static buildpack",
        effect: "routing and clean-URL configuration revert to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- Mobile deep links ---------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("apple-app-site-association"),
        consumer: "iOS, fetched from the domain",
        effect: "breaks Universal Links for ALREADY-SHIPPED apps, unfixable until CDN caches expire",
        failure: FailureMode::ShippedClientsBreak,
    },
    PlatformContract {
        matcher: Matcher::Basename("assetlinks.json"),
        consumer: "Android, fetched from the domain",
        effect: "breaks App Links for ALREADY-SHIPPED apps, unfixable until caches expire",
        failure: FailureMode::ShippedClientsBreak,
    },
    // -- .well-known: specific rows before the catch-all ---------------------
    PlatformContract {
        matcher: Matcher::Inside(".well-known/acme-challenge"),
        consumer: "a certificate authority performing an ACME HTTP-01 challenge",
        effect: "breaks automated TLS renewal; surfaces weeks later as an expired certificate",
        failure: FailureMode::RenewalStopsUntilExpiry,
    },
    PlatformContract {
        matcher: Matcher::Inside(".well-known/pki-validation"),
        consumer: "a certificate authority performing domain validation",
        effect: "breaks certificate issuance and renewal; surfaces as an expired certificate",
        failure: FailureMode::RenewalStopsUntilExpiry,
    },
    PlatformContract {
        matcher: Matcher::Basename("apple-developer-merchantid-domain-association"),
        consumer: "Apple Pay domain verification",
        effect: "Apple Pay stops being offered; payments silently do not happen",
        failure: FailureMode::DeliveryStopsSilently,
    },
    PlatformContract {
        matcher: Matcher::Basename("microsoft-identity-association.json"),
        consumer: "Microsoft Entra ID publisher verification",
        effect: "publisher verification lapses and consent prompts change",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename("openid-configuration"),
        consumer: "every OIDC relying party doing discovery",
        effect: "already-integrated clients stop being able to discover the provider",
        failure: FailureMode::ShippedClientsBreak,
    },
    PlatformContract {
        matcher: Matcher::Basename("change-password"),
        consumer: "password managers following the well-known change-password URL",
        effect: "the change-password flow stops resolving",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("security.txt"),
        consumer: "security researchers and automated disclosure tooling",
        effect: "the vulnerability disclosure contract disappears",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Inside(".well-known"),
        consumer: "an external client following RFC 8615",
        effect: "a well-known URI stops resolving; every .well-known path is a contract by definition",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- Advertising ---------------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("ads.txt"),
        consumer: "ad exchanges enforcing IAB authorized-digital-sellers",
        effect: "ad inventory becomes unauthorized: revenue goes to zero with no error anywhere",
        failure: FailureMode::DeliveryStopsSilently,
    },
    PlatformContract {
        matcher: Matcher::Basename("app-ads.txt"),
        consumer: "ad exchanges enforcing IAB authorized-digital-sellers for apps",
        effect: "in-app ad inventory becomes unauthorized: revenue goes to zero silently",
        failure: FailureMode::DeliveryStopsSilently,
    },
    // -- Web servers ---------------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename(".htaccess"),
        consumer: "Apache httpd, per-directory, at request time",
        effect: "request routing, authentication and redirects revert to the server default",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("web.config"),
        consumer: "IIS, at request time",
        effect: "request routing, authentication and rewrites revert to the server default",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- Crawlers ------------------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("robots.txt"),
        consumer: "search-engine and AI crawlers",
        effect: "crawl and index directives disappear; staging paths become indexable",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- Progressive web apps ------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("manifest.webmanifest"),
        consumer: "browsers, for already-installed PWAs",
        effect: "installability and app identity break for ALREADY-INSTALLED apps",
        failure: FailureMode::ShippedClientsBreak,
    },
    // `manifest.json` is the widest row in the table: it is the PWA manifest,
    // the Chrome extension manifest, and also whatever a dozen build tools
    // choose to emit. It stays because two of those three are read from outside
    // the repository and the cost of the collision is a false KEEP, which is the
    // direction §1.3 says to fail in.
    PlatformContract {
        matcher: Matcher::Basename("manifest.json"),
        consumer: "browsers and extension stores, for already-installed apps",
        effect: "installability, permissions and app identity break for ALREADY-INSTALLED apps",
        failure: FailureMode::ShippedClientsBreak,
    },
    PlatformContract {
        matcher: Matcher::Basename("service-worker.js"),
        consumer: "browsers holding a registered service worker",
        effect: "offline behaviour breaks for ALREADY-INSTALLED apps until the registration expires",
        failure: FailureMode::ShippedClientsBreak,
    },
    // -- Runtime version pins, read by the platform --------------------------
    PlatformContract {
        matcher: Matcher::Basename("VERSION"),
        consumer: "release and packaging tooling outside the repository",
        effect: "the published version reverts to a default or to zero",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".python-version"),
        consumer: "pyenv / uv / the build platform",
        effect: "the interpreter silently becomes whatever the platform defaults to",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".nvmrc"),
        consumer: "nvm / the build platform",
        effect: "the Node version silently becomes whatever the platform defaults to",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".ruby-version"),
        consumer: "rbenv / rvm / the build platform",
        effect: "the Ruby version silently becomes whatever the platform defaults to",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".node-version"),
        consumer: "nodenv / the build platform",
        effect: "the Node version silently becomes whatever the platform defaults to",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".tool-versions"),
        consumer: "asdf / mise",
        effect: "every pinned toolchain version silently becomes the platform default",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("runtime.txt"),
        consumer: "Heroku and compatible buildpacks",
        effect: "the runtime silently becomes the buildpack default",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("Procfile"),
        consumer: "Heroku, Foreman and compatible platforms",
        effect: "the process types the platform starts revert to a guess or to nothing",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename(".buildpacks"),
        consumer: "the build platform's buildpack resolver",
        effect: "the buildpack set changes and the build silently produces a different image",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- Hosting platforms ---------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("Staticfile"),
        consumer: "the Cloud Foundry staticfile buildpack",
        effect: "static hosting configuration reverts to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("app.yaml"),
        consumer: "Google App Engine",
        effect: "runtime, scaling and routing configuration read only by the platform disappears",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("fly.toml"),
        consumer: "Fly.io",
        effect: "regions, services, health checks and scaling revert to defaults",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("render.yaml"),
        consumer: "Render",
        effect: "service definitions, cron jobs and routing read only by the platform disappear",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Basename("railway.json"),
        consumer: "Railway",
        effect: "build and deploy configuration read only by the platform disappears",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    PlatformContract {
        matcher: Matcher::Inside(".platform"),
        consumer: "AWS Elastic Beanstalk",
        effect: "platform hooks and nginx configuration stop being applied",
        failure: FailureMode::PlatformBehaviourChanges,
    },
    // -- git itself ----------------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename(".gitattributes"),
        consumer: "git, on every future checkout, add and diff",
        effect: "removing filter=lfs silently commits raw blobs; removing text=auto rewrites \
                 line endings; removing linguist-generated re-arms this very tool against \
                 vendored trees",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename(".gitignore"),
        consumer: "git, on every future `add`",
        effect: "the next `git add -A` commits .env and every other ignored secret (§6.22)",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    // -- CI quality gates ----------------------------------------------------
    PlatformContract {
        matcher: Matcher::Basename("codecov.yml"),
        consumer: "Codecov, from its own side of the integration",
        effect: "silently changes the thresholds that gate merges — the failure is that CI stops failing",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename(".codecov.yml"),
        consumer: "Codecov, from its own side of the integration",
        effect: "silently changes the thresholds that gate merges — the failure is that CI stops failing",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename(".coveragerc"),
        consumer: "coverage.py, including the run CI grades merges on",
        effect: "silently changes the coverage threshold and the measured file set",
        failure: FailureMode::ControlSilentlyRemoved,
    },
    PlatformContract {
        matcher: Matcher::Basename("sonar-project.properties"),
        consumer: "SonarQube / SonarCloud",
        effect: "silently changes the quality gate that blocks merges",
        failure: FailureMode::ControlSilentlyRemoved,
    },
];

/// §6.11's table, for reports and for tests that check it stays well formed.
pub fn platform_contracts() -> &'static [PlatformContract] {
    PLATFORM_CONTRACTS
}

fn platform_contract_for(rel: &Path) -> Option<&'static PlatformContract> {
    PLATFORM_CONTRACTS.iter().find(|c| c.matches(rel))
}

// ---------------------------------------------------------------------------
// 1m — un-ignored by a `!` negation (§6.17)
// ---------------------------------------------------------------------------

/// The `!` rule that re-included a file, and where it is written.
///
/// The source and line are kept because "we kept this because of a negation" is
/// not actionable and "we kept this because `.gitignore:3` says
/// `!/media/customer/.htaccess`" is: a human can go and read the intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegationUnIgnore {
    source: PathBuf,
    line: u32,
    pattern: String,
}

impl NegationUnIgnore {
    /// The ignore file holding the rule, relative to the working tree — or an
    /// absolute path when git names one, as it does for `core.excludesFile`.
    pub fn source(&self) -> PathBuf {
        self.source.clone()
    }

    /// The 1-based line number of the rule within [`NegationUnIgnore::source`].
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The rule verbatim, including its leading `!`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

// ---------------------------------------------------------------------------
// 1n and 1o — the tool's ledger and the tool's evidence (§6.22)
// ---------------------------------------------------------------------------

/// Whether a tool artifact is a ledger (1n) or evidence (1o).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolArtifactKind {
    /// 1n — the veto list a human wrote and the record of what was removed.
    Ledger,
    /// 1o — what the next run reads to decide anything at all.
    Evidence,
}

impl ToolArtifactKind {
    /// The §9.3 class this kind refuses under.
    pub fn class(self) -> ContractClass {
        match self {
            ToolArtifactKind::Ledger => ContractClass::ToolLedger,
            ToolArtifactKind::Evidence => ContractClass::ToolEvidence,
        }
    }
}

/// A file belonging to the tool's own state or evidence base.
#[derive(Debug)]
pub struct ToolArtifact {
    matcher: Matcher,
    kind: ToolArtifactKind,
    what: &'static str,
    also_canonical_junk: bool,
}

impl ToolArtifact {
    /// Ledger or evidence.
    pub fn kind(&self) -> ToolArtifactKind {
        self.kind
    }

    /// What the file is, in one clause.
    pub fn what(&self) -> &'static str {
        self.what
    }

    /// Whether this path also appears on every canonical junk list ever
    /// written. §6.22's point is that the overlap is total for the evidence
    /// rows, which is exactly why the class has to exist: without it the tool
    /// deletes its own evidence and *nothing detects that the cause of the
    /// missing data was the cleaner itself*.
    pub fn also_canonical_junk(&self) -> bool {
        self.also_canonical_junk
    }

    /// The name, path, directory or extension this row recognises.
    pub fn pattern(&self) -> &'static str {
        self.matcher.describe()
    }
}

/// The tool's own files. Compiled in, never read from configuration: §6.22's
/// "config as an attack path" is precisely that the code's *inputs* are editable
/// data, so the one list that protects the veto list cannot itself be data.
static TOOL_ARTIFACTS: &[ToolArtifact] = &[
    // 1n. The keep manifest is the first entry in its own never-touch list.
    ToolArtifact {
        matcher: Matcher::ExactPath(".judged/keep.toml"),
        kind: ToolArtifactKind::Ledger,
        what: "the keep manifest: the veto list a human wrote, which looks stale by design",
        also_canonical_junk: false,
    },
    ToolArtifact {
        matcher: Matcher::ExactPath(".judged/ledger.jsonl"),
        kind: ToolArtifactKind::Ledger,
        what: "the deletion ledger: the record of what was removed and on whose evidence",
        also_canonical_junk: false,
    },
    ToolArtifact {
        matcher: Matcher::Inside(".judged"),
        kind: ToolArtifactKind::Ledger,
        what: "the tool's own state directory",
        also_canonical_junk: false,
    },
    // 1o. Every row below is simultaneously a canonical junk pattern.
    ToolArtifact {
        matcher: Matcher::Basename(".coverage"),
        kind: ToolArtifactKind::Evidence,
        what: "coverage.py's measurement database",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Basename("coverage.xml"),
        kind: ToolArtifactKind::Evidence,
        what: "Cobertura-format coverage, the form most CI systems publish",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Basename("lcov.info"),
        kind: ToolArtifactKind::Evidence,
        what: "LCOV coverage",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Basename("jacoco.exec"),
        kind: ToolArtifactKind::Evidence,
        what: "JaCoCo's binary execution data",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Extension("profraw"),
        kind: ToolArtifactKind::Evidence,
        what: "LLVM raw profile data, before llvm-profdata merges it",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Extension("profdata"),
        kind: ToolArtifactKind::Evidence,
        what: "merged LLVM profile data",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Extension("gcda"),
        kind: ToolArtifactKind::Evidence,
        what: "gcov arc counts — the half of the pair that holds the measurements",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Extension("gcno"),
        kind: ToolArtifactKind::Evidence,
        what: "gcov notes — useless without the .gcda, and the .gcda useless without it",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Inside(".nyc_output"),
        kind: ToolArtifactKind::Evidence,
        what: "nyc/istanbul's raw coverage output",
        also_canonical_junk: true,
    },
    // A bare `coverage/` segment also matches a *source* directory called
    // `coverage` — coverage.py's own repository is the obvious example. That is
    // a false KEEP on someone's source tree, which is noisy and safe, and the
    // alternative is a false DELETE of the evidence base, which is neither.
    ToolArtifact {
        matcher: Matcher::Inside("coverage"),
        kind: ToolArtifactKind::Evidence,
        what: "the conventional coverage report directory",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Inside("htmlcov"),
        kind: ToolArtifactKind::Evidence,
        what: "coverage.py's HTML report",
        also_canonical_junk: true,
    },
    ToolArtifact {
        matcher: Matcher::Inside(".turbo"),
        kind: ToolArtifactKind::Evidence,
        what: "Turborepo's task cache, named by §6.22 as evidence a cleaner destroys",
        also_canonical_junk: true,
    },
];

fn tool_artifact_for(rel: &Path) -> Option<&'static ToolArtifact> {
    TOOL_ARTIFACTS.iter().find(|a| a.matcher.matches(rel))
}

/// The outcome of checking a proposed run against §6.22's ledger rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanReview {
    /// Nothing in the plan violates 1n or 1o.
    Permitted,
    /// A ledger edit and a deletion in the same run. §6.22: *"pruning it must be
    /// structurally impossible in the same run as any deletion"* — because the
    /// pruning is what makes the deletion look permitted.
    RefusedCoOccurrence { edit: PathBuf, deletion: PathBuf },
    /// The plan proposes deleting the tool's own ledger or evidence.
    RefusedSelfDeletion { path: PathBuf, class: ContractClass },
}

/// Check a proposed run: `ledger_edits` are paths the run would write to the
/// keep manifest or the deletion ledger, `deletions` are the paths it would
/// remove.
///
/// This is deliberately a *structural* rule and not a warning. §6.22's failure
/// is a run that prunes a veto and then acts on the pruned list, and no amount
/// of care inside the classifier prevents it, because by the time the classifier
/// runs the veto is already gone.
///
/// Self-deletion is checked before co-occurrence so a plan that does both is
/// reported against the sharper violation.
pub fn review_plan(ledger_edits: &[PathBuf], deletions: &[PathBuf]) -> PlanReview {
    for deletion in deletions {
        if let Some(artifact) = tool_artifact_for(deletion) {
            return PlanReview::RefusedSelfDeletion {
                path: deletion.clone(),
                class: artifact.kind().class(),
            };
        }
    }
    if let (Some(edit), Some(deletion)) = (ledger_edits.first(), deletions.first()) {
        return PlanReview::RefusedCoOccurrence {
            edit: edit.clone(),
            deletion: deletion.clone(),
        };
    }
    PlanReview::Permitted
}

// ---------------------------------------------------------------------------
// 1p — the unknown defaults to KEEP
// ---------------------------------------------------------------------------

/// How a file's type was determined.
///
/// Three ways, and no fourth. In particular there is no `Text` variant: a file
/// being valid UTF-8 says nothing about what reads it, and treating "it parses
/// as text" as a determination would hand 1p's protection back to exactly the
/// files that need it — a bare `notes`, a `config` with no extension, an
/// operator's checked-in `runbook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeSignal {
    /// A recognised file extension.
    Extension(&'static str),
    /// Recognised leading bytes.
    Magic(&'static str),
    /// A name or location the ecosystem gives a fixed meaning.
    PathName(&'static str),
}

impl TypeSignal {
    /// The recognised name, extension or format label.
    pub fn label(&self) -> &'static str {
        match self {
            TypeSignal::Extension(e) | TypeSignal::Magic(e) | TypeSignal::PathName(e) => e,
        }
    }
}

/// Extensions whose meaning is not in doubt, in six groups: source; markup,
/// style and docs; data and config; images, media and fonts; archives and
/// binaries; models, notebooks and certificates.
///
/// An extension NOT on this list is not a determination — `thing.xyzzy` is as
/// unknown as `thing` — so the cost of the list being short is recall, never
/// safety. That is the direction it should fail in.
const KNOWN_EXTENSIONS: &[&str] = &[
    "rs",
    "py",
    "pyi",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "tsx",
    "go",
    "java",
    "kt",
    "kts",
    "scala",
    "rb",
    "php",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "cs",
    "swift",
    "m",
    "mm",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "cmd",
    "pl",
    "pm",
    "lua",
    "r",
    "jl",
    "ex",
    "exs",
    "erl",
    "hrl",
    "clj",
    "cljs",
    "hs",
    "ml",
    "mli",
    "fs",
    "fsx",
    "dart",
    "vue",
    "svelte",
    "elm",
    "nim",
    "zig",
    "sql",
    "proto",
    "thrift",
    "graphql",
    "gql",
    "tf",
    "hcl",
    "bzl",
    "cmake",
    "gradle",
    "sbt",
    "html",
    "htm",
    "xhtml",
    "css",
    "scss",
    "sass",
    "less",
    "styl",
    "md",
    "markdown",
    "rst",
    "adoc",
    "txt",
    "tex",
    "org",
    "json",
    "jsonc",
    "json5",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "properties",
    "env",
    "xml",
    "csv",
    "tsv",
    "parquet",
    "avro",
    "ndjson",
    "jsonl",
    "lock",
    "plist",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "svg",
    "webp",
    "avif",
    "ico",
    "bmp",
    "tiff",
    "mp3",
    "wav",
    "flac",
    "mp4",
    "webm",
    "mov",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot",
    "zip",
    "tar",
    "gz",
    "bz2",
    "xz",
    "zst",
    "7z",
    "jar",
    "war",
    "so",
    "dylib",
    "dll",
    "exe",
    "a",
    "o",
    "wasm",
    "pdf",
    "class",
    "pyc",
    "whl",
    "deb",
    "rpm",
    "dmg",
    "iso",
    "pt",
    "pth",
    "onnx",
    "safetensors",
    "pkl",
    "h5",
    "npy",
    "npz",
    "ipynb",
    "pem",
    "crt",
    "cer",
    "p12",
    "pfx",
    "jks",
    "keystore",
    "db",
    "sqlite",
    "sqlite3",
];

/// Names the ecosystem gives a fixed meaning, for files that carry no extension
/// at all.
const KNOWN_PATH_NAMES: &[&str] = &[
    "Makefile",
    "GNUmakefile",
    "Dockerfile",
    "Containerfile",
    "Vagrantfile",
    "Jenkinsfile",
    "Rakefile",
    "Gemfile",
    "Guardfile",
    "Brewfile",
    "Justfile",
    "justfile",
    "BUILD",
    "WORKSPACE",
    "LICENSE",
    "LICENCE",
    "COPYING",
    "NOTICE",
    "README",
    "CHANGELOG",
    "AUTHORS",
    "CONTRIBUTING",
    "MAINTAINERS",
    "TODO",
    ".editorconfig",
    ".dockerignore",
    ".npmignore",
    ".prettierignore",
    ".eslintignore",
    ".gitmodules",
    ".gitkeep",
    ".keep",
    "py.typed",
    "go.sum",
    "go.mod",
];

/// Leading-byte signatures, as `(bytes, label)`.
///
/// Long signatures first so a prefix of another cannot shadow it.
const MAGIC: &[(&[u8], &str)] = &[
    (b"SQLite format 3\0", "SQLite"),
    (b"\x89PNG\r\n\x1a\n", "PNG"),
    (b"%PDF-", "PDF"),
    (b"GIF87a", "GIF"),
    (b"GIF89a", "GIF"),
    (b"\x7fELF", "ELF"),
    (b"\xca\xfe\xba\xbe", "Java class"),
    (b"\0asm", "WebAssembly"),
    (b"PK\x03\x04", "ZIP"),
    (b"\xff\xd8\xff", "JPEG"),
    (b"\x1f\x8b", "gzip"),
    (b"BZh", "bzip2"),
    (b"\xfd7zXZ\0", "xz"),
    (b"\x28\xb5\x2f\xfd", "zstd"),
    (b"\xcf\xfa\xed\xfe", "Mach-O"),
    (b"\xce\xfa\xed\xfe", "Mach-O"),
    (b"!<arch>\n", "ar archive"),
    (b"<?xml", "XML"),
    (b"#!", "shebang script"),
];

/// How many leading bytes are read to look for a signature. The longest entry
/// in [`MAGIC`] is 16 bytes; the rest is slack so this number never has to move
/// when a signature is added.
const MAGIC_PROBE_BYTES: usize = 64;

/// Determine a file's type from its path and its leading bytes, or say it
/// cannot be determined.
///
/// `head` is the first [`MAGIC_PROBE_BYTES`] bytes, or empty when the file could
/// not be read — an absent file is not a determined type, which is the correct
/// direction.
fn determine_type(rel: &Path, head: &[u8]) -> Option<TypeSignal> {
    if let Some(name) = rel.file_name().and_then(|n| n.to_str()) {
        if let Some(known) = KNOWN_PATH_NAMES.iter().find(|k| **k == name) {
            return Some(TypeSignal::PathName(known));
        }
        // A platform contract is by definition a file whose type is known: the
        // external reader knows what it is well enough to depend on it.
        if let Some(contract) = platform_contract_for(rel) {
            return Some(TypeSignal::PathName(contract.pattern()));
        }
    }
    if let Some(ext) = rel.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        if let Some(known) = KNOWN_EXTENSIONS.iter().find(|k| ***k == *lower) {
            return Some(TypeSignal::Extension(known));
        }
    }
    for (bytes, label) in MAGIC {
        if head.starts_with(bytes) {
            return Some(TypeSignal::Magic(label));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------

/// One reason Gate 1 classes 1l–1p refuse a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// 1l — read by something outside the repository.
    PlatformContract(&'static PlatformContract),
    /// 1m — re-included into the working tree by a `!` negation.
    NegationUnIgnored(NegationUnIgnore),
    /// 1n / 1o — the tool's own ledger or evidence.
    ToolArtifact(&'static ToolArtifact),
    /// 1p — the type could not be determined.
    UnknownType,
}

impl Refusal {
    /// Which class this refusal belongs to.
    pub fn class(&self) -> ContractClass {
        match self {
            Refusal::PlatformContract(_) => ContractClass::PlatformContract,
            Refusal::NegationUnIgnored(_) => ContractClass::NegationUnIgnored,
            Refusal::ToolArtifact(a) => a.kind().class(),
            Refusal::UnknownType => ContractClass::UnknownType,
        }
    }
}

// `PartialEq` on `&'static PlatformContract` / `&'static ToolArtifact` compares
// the pointed-to values, and neither type derives `PartialEq`; deriving it on
// `Refusal` would therefore not compile. Comparing by address is exactly right
// here — the registries are `static`, so two refusals name the same row iff they
// point at it.
impl PartialEq for PlatformContract {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for PlatformContract {}
impl PartialEq for ToolArtifact {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for ToolArtifact {}

/// What Gate 1 classes 1l–1p decided about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Disposition {
    /// Refused. Not a deletion candidate at any tier, on any evidence: §9.3's
    /// refusals are justified by irreversibility, not by uselessness, so more
    /// evidence of uselessness does not move them.
    NeverTouch,
    /// No 1l–1p class applies and the file's type was determined. Gate 1's other
    /// classes and every later gate still get their say; this is not permission
    /// to delete, it is the absence of an objection from these five rules.
    NoObjection,
}

/// The classification of one path under 1l–1p.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractVerdict {
    path: PathBuf,
    reasons: Vec<Refusal>,
    type_signal: Option<TypeSignal>,
}

impl ContractVerdict {
    /// The repository-relative path this verdict is about.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every reason to refuse, in class order. Several can apply at once —
    /// `/media/customer/.htaccess` is both an Apache routing contract and a file
    /// re-included by a negation — and reporting only the first would make the
    /// verdict depend on evaluation order.
    pub fn reasons(&self) -> &[Refusal] {
        &self.reasons
    }

    /// How the file's type was determined, or `None` when it was not.
    pub fn type_signal(&self) -> Option<TypeSignal> {
        self.type_signal
    }

    /// The decision.
    pub fn disposition(&self) -> Disposition {
        if self.reasons.is_empty() {
            Disposition::NoObjection
        } else {
            Disposition::NeverTouch
        }
    }
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// Gate 1 classes 1l–1p, over one repository.
pub struct ContractGate<'a> {
    repo: &'a Repo,
}

impl<'a> ContractGate<'a> {
    /// Bind the gate to a working tree. The repository is needed for 1m only;
    /// the other four classes are functions of the path and its leading bytes.
    pub fn new(repo: &'a Repo) -> ContractGate<'a> {
        ContractGate { repo }
    }

    /// Classify one path.
    ///
    /// `path` may be absolute or relative to the working tree root. A path that
    /// escapes the working tree is an error, never a verdict: classifying
    /// something outside the repository would answer a question nobody asked
    /// with data we do not have.
    pub fn classify(&self, path: &Path) -> Result<ContractVerdict> {
        let rel = self.relative(path)?;
        let mut reasons = Vec::new();

        // 1l.
        if let Some(contract) = platform_contract_for(&rel) {
            reasons.push(Refusal::PlatformContract(contract));
        }
        // 1m.
        if let Some(negation) = self.negation_un_ignore(&rel)? {
            reasons.push(Refusal::NegationUnIgnored(negation));
        }
        // 1n and 1o.
        if let Some(artifact) = tool_artifact_for(&rel) {
            reasons.push(Refusal::ToolArtifact(artifact));
        }
        // 1p, last, because it is the fallback and it stacks rather than
        // overrides: an undeterminable platform contract must still report 1l.
        let head = self.head_bytes(&rel)?;
        let type_signal = determine_type(&rel, &head);
        if type_signal.is_none() {
            reasons.push(Refusal::UnknownType);
        }

        Ok(ContractVerdict {
            path: rel,
            reasons,
            type_signal,
        })
    }

    /// The deciding ignore rule for `rel`, when it is a `!` negation.
    ///
    /// Runs `git check-ignore -vz --no-index --stdin --non-matching`. Three
    /// things about that command line are deliberate:
    ///
    /// - `--no-index` because without it git answers from the index for tracked
    ///   paths and reports no pattern at all — and every file §6.17 measured is
    ///   tracked, so the question would be unanswerable for exactly the
    ///   population the class exists for. Verified on git 2.50.1.
    /// - `--non-matching` so a path matched by nothing produces a record with an
    ///   empty pattern rather than no record, which keeps "no rule matched"
    ///   distinguishable from "git printed nothing because something went
    ///   wrong" (§6.20).
    /// - `-z` so no filename can be misread, whatever it contains.
    fn negation_un_ignore(&self, rel: &Path) -> Result<Option<NegationUnIgnore>> {
        let path = rel.to_str().ok_or_else(|| {
            Error::Git(format!(
                "path {} is not valid UTF-8; refusing to guess its ignore status",
                rel.display()
            ))
        })?;
        let mut stdin = path.as_bytes().to_vec();
        stdin.push(0);
        let output = self.git(
            &[
                "check-ignore",
                "-vz",
                "--no-index",
                "--stdin",
                "--non-matching",
            ],
            Some(&stdin),
        )?;
        // Exit 1 means "none of the given paths are ignored" — an answer. 128
        // (outside the repository, bad config) is a failure and must not read as
        // "no negation".
        match output.status.code() {
            Some(0) | Some(1) => {}
            _ => {
                return Err(Error::Git(format!(
                    "git check-ignore failed for {} ({}): {}",
                    rel.display(),
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                )))
            }
        }
        // Records are four NUL-terminated fields: source, line, pattern, path.
        let fields: Vec<&[u8]> = output.stdout.split(|b| *b == 0).collect();
        if fields.len() < 4 {
            return Err(Error::Git(format!(
                "git check-ignore returned no verdict for {}",
                rel.display()
            )));
        }
        let pattern = std::str::from_utf8(fields[2])
            .map_err(|_| Error::Git("git check-ignore returned a non-UTF-8 pattern".to_string()))?;
        // An empty pattern means no rule matched: the file is not ignored and
        // was not re-included either, so this is not class 1m.
        if !pattern.starts_with('!') {
            return Ok(None);
        }
        let source = std::str::from_utf8(fields[0])
            .map_err(|_| Error::Git("git check-ignore returned a non-UTF-8 source".to_string()))?;
        let line = std::str::from_utf8(fields[1])
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| {
                Error::Git(format!(
                    "git check-ignore returned an unparseable line number for {}",
                    rel.display()
                ))
            })?;
        Ok(Some(NegationUnIgnore {
            source: PathBuf::from(source),
            line,
            pattern: pattern.to_string(),
        }))
    }

    /// The first [`MAGIC_PROBE_BYTES`] bytes of `rel`, or empty when it does not
    /// exist.
    ///
    /// "Already gone" is a legitimate thing to ask about — a candidate list
    /// outlives the tree it was computed from — and an absent file simply has no
    /// magic bytes, which leaves 1p to keep it. Any *other* I/O failure is
    /// returned: a permission error read as "no signature" would silently move a
    /// file from one class to another.
    fn head_bytes(&self, rel: &Path) -> Result<Vec<u8>> {
        use std::io::Read;
        let absolute = self.repo.root().join(rel);
        // A directory has no leading bytes. Asked first, by metadata, because
        // the alternative is recognising EISDIR from `open` or `read` — which of
        // the two fails is platform-dependent, and the errno number is not
        // something to hard-code.
        match std::fs::metadata(&absolute) {
            Ok(meta) if meta.is_dir() => return Ok(Vec::new()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: absolute,
                    source,
                })
            }
        }
        let mut file = match std::fs::File::open(&absolute) {
            Ok(file) => file,
            // Racing with a deletion between the metadata call and the open is
            // the same answer as never having existed.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: absolute,
                    source,
                })
            }
        };
        let mut buf = vec![0u8; MAGIC_PROBE_BYTES];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(source) => {
                    return Err(Error::Io {
                        path: absolute,
                        source,
                    })
                }
            }
        }
        buf.truncate(filled);
        Ok(buf)
    }

    /// Resolve `path` to a working-tree-relative path, refusing anything that
    /// escapes.
    fn relative(&self, path: &Path) -> Result<PathBuf> {
        let root = self.repo.root();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        // Symlinks are resolved where possible so containment is a real-path
        // check (Gate 0c). `canonicalize` fails on paths that do not exist, and
        // classifying a path that is already gone is legitimate, so fall back to
        // the lexical form and let the component walk below refuse an escape.
        let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        let rel = resolved.strip_prefix(root).map_err(|_| {
            Error::Git(format!(
                "{} is outside the working tree {}",
                path.display(),
                root.display()
            ))
        })?;
        let mut normal = PathBuf::new();
        for component in rel.components() {
            match component {
                Component::Normal(part) => normal.push(part),
                Component::CurDir => {}
                // `..` survives only when `canonicalize` could not run, i.e. the
                // path does not exist. Refuse rather than guess where it lands.
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(Error::Git(format!(
                        "{} does not resolve to a path inside the working tree {}",
                        path.display(),
                        root.display()
                    )))
                }
            }
        }
        if normal.as_os_str().is_empty() {
            return Err(Error::Git(format!(
                "{} is the working tree root, not a candidate path",
                root.display()
            )));
        }
        Ok(normal)
    }

    /// Run `git` in the working tree.
    ///
    /// A non-zero exit is not an error here: `check-ignore` answers through its
    /// exit code, so the caller decides which codes are answers.
    fn git(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<std::process::Output> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let mut cmd = Command::new("git");
        cmd.args(&args)
            .current_dir(self.repo.root())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A cleaner that blocks on a credential prompt mid-scan is
        // indistinguishable from one that has crashed.
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        let mut child = cmd.spawn().map_err(|source| {
            Error::Git(format!(
                "failed to run `git` in {}: {source}",
                self.repo.root().display()
            ))
        })?;
        if let Some(bytes) = stdin {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| Error::Git("git stdin pipe was not created".to_string()))?;
            // The payload is a single path, far below the pipe buffer, so
            // writing before waiting cannot deadlock.
            pipe.write_all(bytes)
                .map_err(|source| Error::Git(format!("failed to write to git stdin: {source}")))?;
            // Dropping closes the pipe; `--stdin` readers wait for EOF.
        }
        child
            .wait_with_output()
            .map_err(|source| Error::Git(format!("failed to wait for git: {source}")))
    }
}
