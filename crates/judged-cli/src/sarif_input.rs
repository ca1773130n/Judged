//! Reading a SARIF log off disk, in either shape it can arrive in.
//!
//! `judged_core::sarif` models a *normalized projection* of SARIF 2.1.0 — the
//! leaves, with `tool.driver`, `artifact.location.uri`, `result.message.text`
//! and `result.locations[].physicalLocation.artifactLocation.uri` already
//! flattened. Its own module docs say so, and say that ingest adapters own the
//! mapping down from the wire shape. Until this module existed there was no such
//! adapter in front of `judged ratchet --sarif`, so the flag accepted only the
//! projection: a log straight out of ruff, vulture or knip was refused as
//! `malformed JSON ... missing field \`name\``, which is what serde says when it
//! looks for `tool.name` and finds `tool.driver`.
//!
//! That message was the problem, not the refusal. The file is valid JSON and
//! valid SARIF; only the shape is wrong, and a reader told "malformed JSON" goes
//! looking for a broken analyzer. The nearest thing to a fix from there is to
//! hand-edit the log into the projection — which means hand-writing
//! `executionSuccessful` and `roles: ["analysisTarget"]`, the two assertions
//! §9.2 exists to make an *adapter* responsible for. A confusing refusal that
//! nudges an operator toward forging the health bit is worse than no refusal.
//!
//! So both shapes are read here, told apart structurally: a run whose `tool`
//! carries a `driver` is wire SARIF, and nothing else is. The two are disjoint —
//! the projection puts `name` directly on `tool` — so the sniff cannot be
//! ambiguous, and a file that is neither still fails as it always did.
//!
//! # What this maps, and what it refuses
//!
//! Mapping is the whole job; **inferring is not**. Every field below either
//! comes from the log or is a default the SARIF specification itself defines.
//! Nothing here reconstructs a value the analyzer declined to state:
//!
//! - `executionSuccessful` is never synthesized from an exit code. §9.2 quotes
//!   the spec's own note that "not all programs exit with an exit code of 0 on
//!   success", with a worked example of `exitCode: 1` beside
//!   `executionSuccessful: true`. Absent means `false` here, exactly as it does
//!   in the projection, because absence is not success (§6.20).
//! - `roles: ["analysisTarget"]` is never synthesized from the paths findings
//!   happen to mention. That set is the §9.2 positive control — the declaration
//!   of what the tool actually opened — and deriving it from results would make
//!   a tool that scanned one file look like a tool that scanned everything it
//!   accused. A log with no `artifacts` array therefore declares no scanned
//!   universe and assesses as degraded, which is the correct reading of a tool
//!   that did the work and then declined to say what it looked at.
//! - A path is never recovered from prose. A location the log spells with
//!   neither a `physicalLocation.artifactLocation.uri` nor an `index` into
//!   `run.artifacts` is refused rather than dropped: silently discarding it
//!   would turn a finding about a file into a finding about the repository.
//! - An absolute URI is refused, in every spelling SARIF carries — POSIX,
//!   scheme, UNC share and drive letter alike. The baseline is committed and
//!   diffed by humans (§9.4), and an entry keyed on
//!   `/Users/someone/checkout/src/lib.rs` or `C:\work\src\lib.rs` matches
//!   nothing on any other machine; it is born rotten. This reader does not get
//!   to assume it runs on the machine that wrote the log, and
//!   `judged_core::fingerprint` already draws the same line for the same
//!   reason.
//! - A suppression with no `status` is refused. SARIF defaults that field to
//!   `accepted`; §5.3 does not, because a suppression nobody reviewed is not
//!   amnesty, and reading one as amnesty is the direction that loses code.
//!
//! Where the specification defines a resolution chain, this follows it, because
//! *not* following it is its own kind of inference — and because refusing a log
//! that stated something perfectly clearly, in a spelling this reader declined
//! to learn, turns a healthy run into no run. `result.level` falls back to the
//! rule's `defaultConfiguration.level` and then to `warning`, `ruleIndex`
//! resolves against `tool.driver.rules`, and `artifactLocation.index` resolves
//! against `run.artifacts`. All three are what the format says the bytes mean.
//!
//! Violations are [`Error::Sarif`], never [`Error::Json`]. The distinction is
//! the one `error.rs` already draws: malformed JSON is a broken document, a
//! contract violation is a well-formed document that says the wrong thing, and
//! they get different remediation.

use std::collections::BTreeMap;

use judged_core::sarif::{
    Artifact, BaselineState, Invocation, Level, Location, Notification, Run, SarifLog, SarifResult,
    Suppression, SuppressionKind, SuppressionStatus, Tool,
};
use judged_core::{Error, Result};
use serde_json::Value;

/// Read one SARIF log, in whichever shape it is written in.
///
/// `context` names the document in every error, because "missing field" without
/// the file it is missing from is unactionable when `--sarif` is repeatable.
pub fn read(text: &str, context: &str) -> Result<SarifLog> {
    let value: Value = serde_json::from_str(text).map_err(|source| Error::Json {
        context: context.to_string(),
        source,
    })?;

    if !is_wire(&value) {
        // Deserialized from the text a second time rather than from the `Value`
        // above: `from_value` has no byte offsets to report, and "missing field
        // `level` at line 12 column 7" is worth one redundant parse of a file
        // that is measured in kilobytes.
        return serde_json::from_str(text).map_err(|source| Error::Json {
            context: context.to_string(),
            source,
        });
    }

    let runs = array(&value, "runs", context)?
        .iter()
        .enumerate()
        .map(|(index, run)| wire_run(run, &format!("{context} runs[{index}]")))
        .collect::<Result<Vec<_>>>()?;
    Ok(SarifLog { runs })
}

/// Wire SARIF nests the analyzer under `tool.driver`; the projection puts its
/// name directly on `tool`. One run carrying a driver is enough: a log mixing
/// the two shapes is malformed under both readings, and the projection branch
/// will say so with a byte offset.
fn is_wire(value: &Value) -> bool {
    value
        .get("runs")
        .and_then(Value::as_array)
        .is_some_and(|runs| runs.iter().any(|run| run.pointer("/tool/driver").is_some()))
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

fn wire_run(run: &Value, at: &str) -> Result<Run> {
    let driver = run
        .pointer("/tool/driver")
        .ok_or_else(|| violation(&format!("{at}.tool"), "has no `driver` object"))?;
    let rules = wire_rules(driver, &format!("{at}.tool.driver"))?;

    // Mapped before the results, because a `physicalLocation` may name its file
    // by index into this table rather than by URI.
    let artifacts: Vec<Artifact> = mapped(run, "artifacts", at, wire_artifact)?;

    Ok(Run {
        tool: Tool {
            name: required_str(driver, "name", &format!("{at}.tool.driver"))?.to_string(),
            version: optional_str(driver, "version", &format!("{at}.tool.driver"))?
                .map(str::to_string),
        },
        invocations: mapped(run, "invocations", at, wire_invocation)?,
        results: mapped(run, "results", at, |result, at| {
            wire_result(result, at, &rules, &artifacts)
        })?,
        // `run.baselineGuid` — the baseline a `result.baselineState` is relative
        // to, which is what the projection's field means. Deliberately *not*
        // `run.automationDetails.guid`: that identifies this run, and reading it
        // here would file every finding against an identifier that names the
        // wrong thing. Absent when the run was not diffed against anything,
        // which is the normal case for a tool judged puts a ratchet in front of.
        baseline_guid: optional_str(run, "baselineGuid", at)?.map(str::to_string),
        artifacts,
    })
}

fn wire_invocation(invocation: &Value, at: &str) -> Result<Invocation> {
    Ok(Invocation {
        // Absent is `false`. Never read from `exitCode`, which the spec itself
        // warns does not mean what a reader expects (§9.2).
        execution_successful: match invocation.get("executionSuccessful") {
            None | Some(Value::Null) => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| violation(at, "`executionSuccessful` is not a boolean"))?,
        },
        tool_execution_notifications: mapped(
            invocation,
            "toolExecutionNotifications",
            at,
            wire_notification,
        )?,
    })
}

fn wire_notification(notification: &Value, at: &str) -> Result<Notification> {
    Ok(Notification {
        // SARIF defaults a notification's level to `warning`, and warning is a
        // degradation here. Defaulting the other way would let a tool announce
        // that it stopped covering part of the repository and have it read as
        // routine chatter.
        level: match notification.get("level") {
            None | Some(Value::Null) => Level::Warning,
            Some(value) => wire_level(value, at)?,
        },
        message: message_text(notification, at)?,
    })
}

fn wire_artifact(artifact: &Value, at: &str) -> Result<Artifact> {
    let uri = artifact
        .pointer("/location/uri")
        .ok_or_else(|| violation(at, "has no `location.uri`"))?;
    Ok(Artifact {
        location_uri: repo_relative_uri(uri, &format!("{at}.location"))?.to_string(),
        roles: match artifact.get("roles") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or_else(|| violation(at, "`roles` is not an array"))?
                .iter()
                .map(|role| {
                    role.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| violation(at, "a member of `roles` is not a string"))
                })
                .collect::<Result<Vec<_>>>()?,
        },
    })
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// `tool.driver.rules`, reduced to what a result needs from it: the id a
/// `ruleIndex` resolves to, and the severity a result inherits when it states
/// none.
struct Rules(Vec<(Option<String>, Option<Level>)>);

impl Rules {
    fn by_index(&self, index: usize) -> Option<&(Option<String>, Option<Level>)> {
        self.0.get(index)
    }

    fn level_of(&self, rule_id: &str) -> Option<Level> {
        self.0
            .iter()
            .find(|(id, _)| id.as_deref() == Some(rule_id))
            .and_then(|(_, level)| *level)
    }
}

fn wire_rules(driver: &Value, at: &str) -> Result<Rules> {
    let rules = match driver.get("rules") {
        None | Some(Value::Null) => return Ok(Rules(Vec::new())),
        Some(value) => value
            .as_array()
            .ok_or_else(|| violation(at, "`rules` is not an array"))?,
    };
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let at = format!("{at}.rules[{index}]");
            let level = match rule.pointer("/defaultConfiguration/level") {
                None | Some(Value::Null) => None,
                Some(value) => Some(wire_level(value, &at)?),
            };
            Ok((optional_str(rule, "id", &at)?.map(str::to_string), level))
        })
        .collect::<Result<Vec<_>>>()
        .map(Rules)
}

fn wire_result(
    result: &Value,
    at: &str,
    rules: &Rules,
    artifacts: &[Artifact],
) -> Result<SarifResult> {
    let by_index = match result.get("ruleIndex") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let index = value
                .as_u64()
                .ok_or_else(|| violation(at, "`ruleIndex` is not a non-negative integer"))?;
            let index = usize::try_from(index)
                .map_err(|_| violation(at, "`ruleIndex` is larger than this platform's usize"))?;
            Some(rules.by_index(index).ok_or_else(|| {
                violation(
                    at,
                    &format!("`ruleIndex` {index} does not name a rule in `tool.driver.rules`"),
                )
            })?)
        }
    };

    let rule_id = match optional_str(result, "ruleId", at)? {
        Some(id) => id.to_string(),
        None => by_index
            .and_then(|(id, _)| id.clone())
            .ok_or_else(|| violation(at, "has neither a `ruleId` nor a `ruleIndex` that resolves to a rule with an `id`, so the finding cannot be identified across runs"))?,
    };

    // The specification's own chain: the result's level, then the rule's
    // configured default, then `warning`.
    let level = match result.get("level") {
        Some(value) if !value.is_null() => wire_level(value, at)?,
        _ => rules
            .level_of(&rule_id)
            .or_else(|| by_index.and_then(|(_, level)| *level))
            .unwrap_or(Level::Warning),
    };

    Ok(SarifResult {
        rule_id,
        level,
        message: message_text(result, at)?,
        locations: mapped(result, "locations", at, |location, at| {
            wire_location(location, at, artifacts)
        })?,
        partial_fingerprints: wire_fingerprints(result, at)?,
        baseline_state: match result.get("baselineState") {
            None | Some(Value::Null) => None,
            Some(value) => Some(wire_baseline_state(value, at)?),
        },
        suppressions: mapped(result, "suppressions", at, wire_suppression)?,
    })
}

fn wire_location(location: &Value, at: &str, artifacts: &[Artifact]) -> Result<Location> {
    let at_location = format!("{at}.physicalLocation.artifactLocation");
    let artifact_location = location.pointer("/physicalLocation/artifactLocation");

    // A conforming log may spell the file either way: inline as `uri`, or as an
    // `index` into `run.artifacts`, which is how a tool avoids repeating a long
    // path on every finding. Refusing the indexed form would reject logs that
    // are perfectly explicit about the path — turning a healthy run into no run,
    // which §6.20 names as the failure that ends in mass deletion. `uri` wins
    // when a log carries both.
    let uri = match artifact_location.and_then(|location| location.get("uri")) {
        Some(uri) => repo_relative_uri(uri, &at_location)?.to_string(),
        None => match artifact_location.and_then(|location| location.get("index")) {
            Some(index) => artifact_by_index(index, &at_location, artifacts)?,
            None => {
                return Err(violation(
                    at,
                    "has no `physicalLocation.artifactLocation.uri` and no `index` into \
                     `run.artifacts`. A location this reader cannot resolve to a path is refused \
                     rather than dropped: a discarded location turns a finding about a file into a \
                     finding about the repository",
                ))
            }
        },
    };

    Ok(Location {
        uri,
        // Display only. §9.2: fingerprints are content-derived and never
        // line-based, or every reformat resets the stability clock.
        start_line: match location.pointer("/physicalLocation/region/startLine") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|line| u32::try_from(line).ok())
                    .ok_or_else(|| violation(at, "`region.startLine` is not a line number"))?,
            ),
        },
    })
}

/// Resolve `artifactLocation.index` against the run's artifact table.
///
/// An index naming nothing is refused rather than dropped, for the same reason a
/// missing URI is: the log stated which file it meant and this reader could not
/// follow it, which is not the same as a finding about no file in particular.
fn artifact_by_index(index: &Value, at: &str, artifacts: &[Artifact]) -> Result<String> {
    let index = index
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| violation(at, "`index` is not a non-negative integer"))?;

    artifacts
        .get(index)
        .map(|artifact| artifact.location_uri.clone())
        .ok_or_else(|| {
            violation(
                at,
                &format!(
                    "`index` {index} does not name an entry in `run.artifacts`, which holds {}",
                    match artifacts.len() {
                        0 => "nothing — the run declared no artifacts at all".to_string(),
                        1 => "one entry".to_string(),
                        count => format!("{count} entries"),
                    }
                ),
            )
        })
}

fn wire_fingerprints(result: &Value, at: &str) -> Result<BTreeMap<String, String>> {
    let object = match result.get("partialFingerprints") {
        None | Some(Value::Null) => return Ok(BTreeMap::new()),
        Some(value) => value
            .as_object()
            .ok_or_else(|| violation(at, "`partialFingerprints` is not an object"))?,
    };
    object
        .iter()
        .map(|(algorithm, value)| {
            let value = value.as_str().ok_or_else(|| {
                violation(
                    at,
                    &format!("partial fingerprint `{algorithm}` is not a string"),
                )
            })?;
            Ok((algorithm.clone(), value.to_string()))
        })
        .collect()
}

fn wire_suppression(suppression: &Value, at: &str) -> Result<Suppression> {
    let kind = match required_str(suppression, "kind", at)? {
        "inSource" => SuppressionKind::InSource,
        "external" => SuppressionKind::External,
        other => {
            return Err(violation(
                at,
                &format!("`kind` is `{other}`; SARIF defines `inSource` and `external`"),
            ))
        }
    };

    // SARIF defaults this to `accepted`. Taking that default would hand amnesty
    // to a suppression nobody stated a review outcome for, and §5.3 is explicit
    // that `underReview` and `rejected` are not amnesty. An absent status is the
    // ambiguity §6.20 forbids resolving in the tool's favour, so it is refused.
    let status =
        match optional_str(suppression, "status", at)? {
            Some("accepted") => SuppressionStatus::Accepted,
            Some("underReview") => SuppressionStatus::UnderReview,
            Some("rejected") => SuppressionStatus::Rejected,
            Some(other) => {
                return Err(violation(
                    at,
                    &format!(
                    "suppression `status` is `{other}`; SARIF defines `accepted`, `underReview` \
                     and `rejected`"
                ),
                ))
            }
            None => return Err(violation(
                at,
                "a suppression with no `status`. SARIF defaults it to `accepted`; §5.3 does not, \
                 because a suppression nobody reviewed is not amnesty. State it explicitly",
            )),
        };

    Ok(Suppression {
        kind,
        status,
        justification: optional_str(suppression, "justification", at)?.map(str::to_string),
    })
}

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

fn wire_level(value: &Value, at: &str) -> Result<Level> {
    match value.as_str() {
        Some("none") => Ok(Level::None),
        Some("note") => Ok(Level::Note),
        Some("warning") => Ok(Level::Warning),
        Some("error") => Ok(Level::Error),
        // Loudly, on purpose. Quietly demoting a level this reader does not
        // recognize would make a fatal notification invisible.
        _ => Err(violation(
            at,
            "`level` must be one of `none`, `note`, `warning`, `error`",
        )),
    }
}

fn wire_baseline_state(value: &Value, at: &str) -> Result<BaselineState> {
    match value.as_str() {
        Some("new") => Ok(BaselineState::New),
        Some("unchanged") => Ok(BaselineState::Unchanged),
        Some("updated") => Ok(BaselineState::Updated),
        Some("absent") => Ok(BaselineState::Absent),
        _ => Err(violation(
            at,
            "`baselineState` must be one of `new`, `unchanged`, `updated`, `absent`",
        )),
    }
}

/// `message.text`, required. SARIF also permits `message.id` against the tool's
/// `globalMessageStrings`; resolving one is not attempted, because a finding
/// whose text this reader had to invent is a finding a human cannot review.
fn message_text(value: &Value, at: &str) -> Result<String> {
    match value.pointer("/message/text").and_then(Value::as_str) {
        Some(text) => Ok(text.to_string()),
        None => Err(violation(
            at,
            "has no `message.text`. A `message.id` against `globalMessageStrings` is not resolved \
             here: a finding whose text was invented by the reader is one nobody can review",
        )),
    }
}

/// The projection's URIs are repo-relative, because the baseline is committed
/// and read on other machines.
fn repo_relative_uri<'a>(value: &'a Value, at: &str) -> Result<&'a str> {
    let uri = value
        .as_str()
        .ok_or_else(|| violation(at, "`uri` is not a string"))?;

    if is_absolute(uri) {
        return Err(violation(
            at,
            &format!(
                "`uri` is `{uri}`, which is absolute. The baseline is committed and diffed by \
                 humans (§9.4), so an entry keyed on one machine's checkout path matches nothing \
                 anywhere else. Emit repository-relative URIs"
            ),
        ));
    }
    Ok(uri)
}

/// Rooted outside the repository, in any of the spellings SARIF actually
/// carries.
///
/// The Windows forms are not hypothetical, and this reader does not get to
/// assume it runs on the machine that produced the log: SARIF is a
/// cross-platform interchange format, so a log written on Windows is routinely
/// judged on Linux. `judged_core::fingerprint` already draws exactly this
/// distinction for the same reason — a drive-lettered path is as
/// checkout-specific as a POSIX one — and the two must not disagree about what
/// counts as absolute.
fn is_absolute(uri: &str) -> bool {
    let bytes = uri.as_bytes();

    // POSIX root, a UNC share (`\\server\share`), and a Windows root-relative
    // path (`\repo\src`).
    if bytes
        .first()
        .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
    {
        return true;
    }
    // A scheme: `file:///home/...`, `https://...`.
    if uri.contains("://") {
        return true;
    }
    // A drive letter: `C:\repo\src` or `C:/repo/src`.
    bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

fn array<'a>(value: &'a Value, key: &str, at: &str) -> Result<&'a Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| violation(at, &format!("`{key}` is missing or is not an array")))
}

/// Map an optional array member-wise, numbering each member in its own errors.
fn mapped<T>(
    value: &Value,
    key: &str,
    at: &str,
    each: impl Fn(&Value, &str) -> Result<T>,
) -> Result<Vec<T>> {
    let members = match value.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .ok_or_else(|| violation(at, &format!("`{key}` is not an array")))?,
    };
    members
        .iter()
        .enumerate()
        .map(|(index, member)| each(member, &format!("{at}.{key}[{index}]")))
        .collect()
}

fn required_str<'a>(value: &'a Value, key: &str, at: &str) -> Result<&'a str> {
    optional_str(value, key, at)?.ok_or_else(|| violation(at, &format!("has no `{key}`")))
}

fn optional_str<'a>(value: &'a Value, key: &str, at: &str) -> Result<Option<&'a str>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| violation(at, &format!("`{key}` is not a string"))),
        )
        .transpose(),
    }
}

fn violation(at: &str, what: &str) -> Error {
    Error::Sarif(format!("{at} {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIRE: &str = r#"{
      "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
      "version": "2.1.0",
      "runs": [{
        "tool": { "driver": {
          "name": "vulture",
          "version": "2.14",
          "rules": [{ "id": "unused-function", "defaultConfiguration": { "level": "error" } }]
        }},
        "invocations": [{ "executionSuccessful": true, "exitCode": 1 }],
        "artifacts": [{ "location": { "uri": "app/dead.py" }, "roles": ["analysisTarget"] }],
        "results": [{
          "ruleId": "unused-function",
          "message": { "text": "unused function 'legacy'" },
          "locations": [{ "physicalLocation": {
            "artifactLocation": { "uri": "app/dead.py" },
            "region": { "startLine": 12 }
          }}]
        }]
      }]
    }"#;

    fn wire_with(patch: impl Fn(&mut Value)) -> String {
        let mut value: Value = serde_json::from_str(WIRE).expect("fixture parses");
        patch(&mut value);
        value.to_string()
    }

    fn refusal(text: &str) -> String {
        match read(text, "log.sarif") {
            Err(Error::Sarif(message)) => message,
            Err(other) => panic!("expected a contract violation, got {other}"),
            Ok(_) => panic!("expected a refusal"),
        }
    }

    #[test]
    fn wire_sarif_maps_onto_the_projection() {
        let log = read(WIRE, "log.sarif").expect("wire SARIF is readable");
        let run = &log.runs[0];

        assert_eq!(run.tool.name, "vulture");
        assert_eq!(run.tool.version.as_deref(), Some("2.14"));
        assert!(run.invocations[0].execution_successful);
        assert_eq!(run.artifacts[0].location_uri, "app/dead.py");
        assert_eq!(run.artifacts[0].roles, vec!["analysisTarget".to_string()]);
        assert_eq!(run.results[0].rule_id, "unused-function");
        assert_eq!(run.results[0].message, "unused function 'legacy'");
        assert_eq!(run.results[0].locations[0].uri, "app/dead.py");
        assert_eq!(run.results[0].locations[0].start_line, Some(12));
    }

    #[test]
    fn the_projection_still_reads_exactly_as_before() {
        let projection = r#"{"runs":[{
          "tool": { "name": "vulture" },
          "invocations": [{ "executionSuccessful": true }],
          "artifacts": [{ "locationUri": "app/dead.py", "roles": ["analysisTarget"] }],
          "results": [{
            "ruleId": "unused-function",
            "level": "warning",
            "message": "unused function 'legacy'",
            "locations": [{ "uri": "app/dead.py", "startLine": 12 }]
          }]
        }]}"#;

        let log = read(projection, "log.sarif").expect("the projection is still readable");
        assert_eq!(log.runs[0].tool.name, "vulture");
        assert_eq!(log.runs[0].results[0].level, Level::Warning);
    }

    #[test]
    fn a_wire_log_is_a_contract_violation_not_malformed_json() {
        // The whole reason this module exists: the file below is valid JSON and
        // valid SARIF, and the old reader called it malformed.
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["tool"]["driver"]
                .as_object_mut()
                .expect("driver is an object")
                .remove("name");
        }));
        assert!(message.contains("tool.driver"), "{message}");
        assert!(message.contains("has no `name`"), "{message}");
    }

    #[test]
    fn execution_successful_is_never_synthesized_from_an_exit_code() {
        let log = read(
            &wire_with(|value| {
                value["runs"][0]["invocations"][0] = serde_json::json!({ "exitCode": 0 });
            }),
            "log.sarif",
        )
        .expect("a log with no health bit still parses");
        assert!(
            !log.runs[0].invocations[0].execution_successful,
            "absence is not success (§6.20)"
        );
    }

    #[test]
    fn analysis_targets_are_never_synthesized_from_result_locations() {
        let log = read(
            &wire_with(|value| {
                value["runs"][0]
                    .as_object_mut()
                    .expect("run is an object")
                    .remove("artifacts");
            }),
            "log.sarif",
        )
        .expect("a log with no artifacts array still parses");
        assert!(
            log.runs[0].artifacts.is_empty(),
            "a tool that declared no scanned universe must not appear to have declared one"
        );
    }

    #[test]
    fn level_falls_back_to_the_rules_default_then_to_warning() {
        let inherited = read(WIRE, "log.sarif").expect("readable");
        assert_eq!(
            inherited.runs[0].results[0].level,
            Level::Error,
            "the rule's defaultConfiguration.level is what the format says the bytes mean"
        );

        let bare = read(
            &wire_with(|value| {
                value["runs"][0]["tool"]["driver"]
                    .as_object_mut()
                    .expect("driver is an object")
                    .remove("rules");
            }),
            "log.sarif",
        )
        .expect("readable");
        assert_eq!(bare.runs[0].results[0].level, Level::Warning);
    }

    #[test]
    fn a_rule_index_resolves_to_the_rules_id() {
        let log = read(
            &wire_with(|value| {
                let result = value["runs"][0]["results"][0]
                    .as_object_mut()
                    .expect("result is an object");
                result.remove("ruleId");
                result.insert("ruleIndex".to_string(), serde_json::json!(0));
            }),
            "log.sarif",
        )
        .expect("readable");
        assert_eq!(log.runs[0].results[0].rule_id, "unused-function");
    }

    #[test]
    fn a_finding_with_no_identity_is_refused() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]
                .as_object_mut()
                .expect("result is an object")
                .remove("ruleId");
        }));
        assert!(message.contains("ruleId"), "{message}");
    }

    #[test]
    fn a_message_this_reader_would_have_to_invent_is_refused() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]["message"] = serde_json::json!({ "id": "unusedFunc" });
        }));
        assert!(message.contains("message.text"), "{message}");
    }

    #[test]
    fn a_location_with_no_path_is_refused_rather_than_dropped() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]["locations"][0] =
                serde_json::json!({ "logicalLocations": [{ "name": "legacy" }] });
        }));
        assert!(message.contains("physicalLocation"), "{message}");
    }

    #[test]
    fn an_absolute_uri_is_refused_in_every_spelling() {
        // A log written on Windows is routinely judged on Linux, so this check
        // cannot assume the running machine's separator.
        // `judged_core::fingerprint` draws the same line; they must not
        // disagree about what counts as checkout-specific.
        for absolute in [
            "/Users/someone/checkout/app/dead.py",
            "file:///Users/someone/checkout/app/dead.py",
            r"C:\work\app\dead.py",
            "C:/work/app/dead.py",
            r"\\build-server\share\app\dead.py",
            r"\work\app\dead.py",
        ] {
            let message = refusal(&wire_with(|value| {
                value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                    ["artifactLocation"]["uri"] = serde_json::json!(absolute);
            }));
            assert!(message.contains("absolute"), "{absolute}: {message}");
        }
    }

    #[test]
    fn a_relative_uri_that_merely_looks_windowsy_is_kept() {
        // The drive-letter check must not eat a real repo-relative path: a
        // colon is a legal character in a POSIX filename, and one letter before
        // the separator is what makes a drive letter a drive letter.
        for relative in ["app/dead.py", "go:generate/main.go", "ab:/c.py"] {
            let log = read(
                &wire_with(|value| {
                    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                        ["artifactLocation"]["uri"] = serde_json::json!(relative);
                }),
                "log.sarif",
            )
            .unwrap_or_else(|error| panic!("{relative} should be readable: {error}"));
            assert_eq!(log.runs[0].results[0].locations[0].uri, relative);
        }
    }

    #[test]
    fn a_location_named_by_artifact_index_resolves_rather_than_refusing() {
        // The indexed form is how a tool avoids repeating a long path on every
        // finding. It states the file exactly; refusing it would turn a healthy
        // run into no run (§6.20).
        let log = read(
            &wire_with(|value| {
                value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                    ["artifactLocation"] = serde_json::json!({ "index": 0 });
            }),
            "log.sarif",
        )
        .expect("an indexed location is readable");
        assert_eq!(log.runs[0].results[0].locations[0].uri, "app/dead.py");
    }

    #[test]
    fn an_artifact_index_naming_nothing_is_refused() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                ["artifactLocation"] = serde_json::json!({ "index": 7 });
        }));
        assert!(message.contains("does not name an entry"), "{message}");
    }

    #[test]
    fn the_baseline_guid_is_the_baseline_not_this_run() {
        // `automationDetails.guid` identifies the run; `baselineGuid` identifies
        // what `baselineState` is relative to. Reading the first as the second
        // files every finding against an identifier that names the wrong thing.
        let log = read(
            &wire_with(|value| {
                let run = value["runs"][0].as_object_mut().expect("run is an object");
                run.insert(
                    "automationDetails".to_string(),
                    serde_json::json!({ "guid": "this-run" }),
                );
                run.insert(
                    "baselineGuid".to_string(),
                    serde_json::json!("the-baseline"),
                );
            }),
            "log.sarif",
        )
        .expect("readable");
        assert_eq!(log.runs[0].baseline_guid.as_deref(), Some("the-baseline"));
    }

    #[test]
    fn an_unknown_level_fails_loudly() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]["level"] = serde_json::json!("fatal");
        }));
        assert!(
            message.contains("`none`, `note`, `warning`, `error`"),
            "{message}"
        );
    }

    #[test]
    fn a_suppression_with_no_status_is_not_amnesty() {
        let message = refusal(&wire_with(|value| {
            value["runs"][0]["results"][0]["suppressions"] =
                serde_json::json!([{ "kind": "inSource" }]);
        }));
        assert!(message.contains("not amnesty"), "{message}");
    }

    #[test]
    fn a_reviewed_suppression_is_carried() {
        let log = read(
            &wire_with(|value| {
                value["runs"][0]["results"][0]["suppressions"] = serde_json::json!([{
                    "kind": "inSource",
                    "status": "underReview",
                    "justification": "pending an owner"
                }]);
            }),
            "log.sarif",
        )
        .expect("readable");
        let suppression = &log.runs[0].results[0].suppressions[0];
        assert_eq!(suppression.kind, SuppressionKind::InSource);
        assert_eq!(suppression.status, SuppressionStatus::UnderReview);
        assert_eq!(
            suppression.justification.as_deref(),
            Some("pending an owner")
        );
    }

    #[test]
    fn a_notification_with_no_level_degrades_rather_than_reads_as_chatter() {
        let log = read(
            &wire_with(|value| {
                value["runs"][0]["invocations"][0]["toolExecutionNotifications"] =
                    serde_json::json!([{ "message": { "text": "could not load vite.config.ts" } }]);
            }),
            "log.sarif",
        )
        .expect("readable");
        let notification = &log.runs[0].invocations[0].tool_execution_notifications[0];
        assert_eq!(notification.level, Level::Warning);
        assert_eq!(notification.message, "could not load vite.config.ts");
    }

    #[test]
    fn genuinely_malformed_json_is_still_malformed_json() {
        match read("{\"runs\": [", "log.sarif") {
            Err(Error::Json { context, .. }) => assert_eq!(context, "log.sarif"),
            other => panic!("expected a JSON error, got {other:?}"),
        }
    }
}
