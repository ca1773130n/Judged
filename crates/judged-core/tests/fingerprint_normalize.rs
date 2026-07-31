//! `normalize_message` erases the parts of a diagnostic that describe the *run*
//! rather than the *finding*.
//!
//! §9.2 requires content-derived identity, and the message is one of the inputs
//! (§9.4 `CANDIDATE`). Tools routinely bake positions, checkout-absolute paths
//! and timings into the message string, so hashing the raw text would reset the
//! stability clock on every reformat, every CI runner with a different workspace
//! root, and every run. Message shapes below are the real ones emitted by ruff
//! and knip, the two adapters §9.2 calls out by name.

use judged_core::fingerprint::normalize_message;

#[test]
fn ruff_concise_position_prefix_is_erased() {
    // `ruff check --output-format concise` prints `<path>:<line>:<col>: <code> <text>`.
    let at_12 = normalize_message("src/pkg/models.py:12:5: F401 `os` imported but unused");
    let at_341 = normalize_message("src/pkg/models.py:341:9: F401 `os` imported but unused");

    assert_eq!(
        at_12, at_341,
        "a reformat moved the import; the finding did not"
    );
    assert_eq!(at_12, "src/pkg/models.py: F401 `os` imported but unused");
}

#[test]
fn knip_absolute_path_is_erased_so_two_checkouts_agree() {
    // knip resolves through the tsconfig and prints workspace-absolute paths, so
    // the same finding reads differently on a laptop and on a CI runner.
    let laptop = normalize_message(
        "Unused export `formatDate` (function) at /Users/neo/dev/app/src/utils/date.ts:12:14",
    );
    let ci = normalize_message(
        "Unused export `formatDate` (function) at /home/runner/work/app/src/utils/date.ts:41:2",
    );

    assert_eq!(laptop, ci);
    assert_eq!(laptop, "Unused export `formatDate` (function) at <path>");
}

#[test]
fn windows_absolute_path_is_erased() {
    // SARIF is a cross-platform interchange format; a drive-lettered path is as
    // checkout-specific as a POSIX one.
    let normalized = normalize_message(r"Unused file C:\work\app\src\utils\date.ts:12:14");
    assert_eq!(normalized, "Unused file <path>");
}

#[test]
fn repo_relative_paths_are_preserved() {
    // A repo-relative path is stable across checkouts and is part of what the
    // finding is *about* — erasing it would merge distinct findings.
    let normalized = normalize_message("Unused file src/legacy/report.ts");
    assert_eq!(normalized, "Unused file src/legacy/report.ts");
}

#[test]
fn parenthesised_position_suffixes_are_erased() {
    let first = normalize_message("`Widget` is never referenced (src/pkg/models.py:10:5)");
    let second = normalize_message("`Widget` is never referenced (src/pkg/models.py:400:1)");

    assert_eq!(first, second);
    assert_eq!(first, "`Widget` is never referenced (src/pkg/models.py)");
}

#[test]
fn word_form_line_and_column_numbers_are_erased() {
    // Go's `deadcode` and tsc-shaped adapters spell positions out in prose.
    let first = normalize_message("declared and not used at line 12, column 5");
    let second = normalize_message("declared and not used at line 908, column 41");

    assert_eq!(first, second);
    assert_eq!(first, "declared and not used at line <n>, column <n>");
}

#[test]
fn attached_durations_are_erased() {
    let fast = normalize_message("Checked 512 files in 1.234s");
    let slow = normalize_message("Checked 512 files in 0.087s");

    assert_eq!(fast, slow);
    assert_eq!(fast, "Checked 512 files in <dur>");
    assert_eq!(normalize_message("resolved in 3m12s"), "resolved in <dur>");
    assert_eq!(normalize_message("resolved in 450ms"), "resolved in <dur>");
}

#[test]
fn spaced_durations_are_erased() {
    let first = normalize_message("resolved in 45 ms");
    let second = normalize_message("resolved in 3200 ms");

    assert_eq!(first, second);
    assert_eq!(first, "resolved in <dur>");
    assert_eq!(normalize_message("waited 2.5 seconds"), "waited <dur>");
}

#[test]
fn wrapping_and_indentation_are_collapsed() {
    let wrapped =
        normalize_message("  F841 Local variable `x`\n\tis assigned to\n  but never used  ");
    assert_eq!(
        wrapped,
        "F841 Local variable `x` is assigned to but never used"
    );
}

#[test]
fn genuinely_different_messages_stay_different() {
    // The normalizer must not be so aggressive that it merges accusations.
    assert_ne!(
        normalize_message("`os` imported but unused"),
        normalize_message("`sys` imported but unused")
    );
    assert_ne!(
        normalize_message("Unused export `formatDate`"),
        normalize_message("Unused export `parseDate`")
    );
    assert_ne!(
        normalize_message("Unused file src/a.ts"),
        normalize_message("Unused file src/b.ts")
    );
}

#[test]
fn normalization_is_idempotent() {
    // The placeholders it emits must survive a second pass unchanged, otherwise
    // a normalized message stored in a baseline would drift when re-normalized.
    for raw in [
        "src/pkg/models.py:12:5: F401 `os` imported but unused",
        "Unused export `formatDate` (function) at /Users/neo/dev/app/src/utils/date.ts:12:14",
        "declared and not used at line 12, column 5",
        "Checked 512 files in 1.234s",
        "resolved in 45 ms",
        "",
    ] {
        let once = normalize_message(raw);
        let twice = normalize_message(&once);
        assert_eq!(once, twice, "not idempotent for {raw:?}");
    }
}

#[test]
fn empty_and_whitespace_only_messages_normalize_to_empty() {
    assert_eq!(normalize_message(""), "");
    assert_eq!(normalize_message("   \n\t "), "");
}
