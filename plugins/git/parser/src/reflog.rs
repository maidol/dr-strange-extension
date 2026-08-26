//! The reflog — and the rebases only it remembers.
//!
//! A rebase leaves **no trace in the commit graph**. It writes new commits and
//! moves a branch; the old commits are simply no longer reachable, and nothing
//! in any object says one replaced the other. The reflog is the only record,
//! and it is local to one clone and expires (`gc.reflogExpire`, 90 days by
//! default) — so a rebase older than that is unrecoverable, by anyone, from
//! anywhere. Everything this module produces is qualified by that, and the
//! plugin says so in its report rather than letting an absent `Rebase` node
//! read as "no rebase happened".
//!
//! The format is one line per ref update:
//!
//! ```text
//! <old-oid> <new-oid> <name> <email> <ts> <tz>\t<message>
//! ```
//!
//! Both oids on every line is what makes the reconstruction possible: the
//! commit a rebase *replaced* is the `old` of the entry that started it.

use std::collections::BTreeMap;

use crate::Files;
use crate::object::Sig;

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// The ref this update was on: `HEAD`, or a full refname.
    pub refname: String,
    pub old: String,
    pub new: String,
    pub who: Sig,
    /// What git called the operation, e.g. `rebase (pick): feat one`.
    pub message: String,
}

/// Every reflog in the repository, keyed by ref, each in the order written
/// (oldest first — which is the order a replay happened in).
pub fn read_logs(files: &dyn Files, listing: &[String]) -> BTreeMap<String, Vec<LogEntry>> {
    let mut out = BTreeMap::new();
    for path in listing.iter().filter(|p| p.starts_with("logs/")) {
        let refname = path.trim_start_matches("logs/").to_string();
        let Ok(bytes) = files.read(path) else {
            continue;
        };
        let entries = parse_log(&refname, &String::from_utf8_lossy(&bytes));
        if !entries.is_empty() {
            out.insert(refname, entries);
        }
    }
    out
}

pub fn parse_log(refname: &str, text: &str) -> Vec<LogEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        // The tab is the one unambiguous boundary: a name may contain spaces,
        // and so may the message.
        let (fields, message) = match line.split_once('\t') {
            Some((f, m)) => (f, m),
            None => (line, ""),
        };
        let mut parts = fields.splitn(3, ' ');
        let (Some(old), Some(new), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if !crate::refs::is_oid(old) || !crate::refs::is_oid(new) {
            continue;
        }
        out.push(LogEntry {
            refname: refname.to_string(),
            old: old.to_string(),
            new: new.to_string(),
            who: Sig::parse(rest),
            message: message.to_string(),
        });
    }
    out
}

/// One replay: what it started from, what it wrote, and what it displaced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rebase {
    /// The branch that was rewritten, when the run named one. A rebase on a
    /// detached HEAD names none, and this stays empty.
    pub refname: String,
    pub interactive: bool,
    /// The commit the replay was based on.
    pub onto: String,
    /// The tip that was rewritten away — the reason this is worth recording,
    /// since nothing else in the repository remembers it.
    pub replaced: String,
    /// The tip the branch ended on.
    pub result: String,
    pub produced: Vec<Step>,
    pub started_ts: i64,
    pub started_at: String,
    pub finished_ts: i64,
    pub finished_at: String,
    /// Whether a `finish` entry closed the run. False means it was aborted or
    /// is still going — which is worth recording, not worth hiding.
    pub completed: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    /// `pick`, `squash`, `fixup`, `reword`… as the reflog named it.
    pub kind: String,
    pub oid: String,
    pub subject: String,
}

/// What a `rebase…` reflog message is doing.
#[derive(Debug, PartialEq)]
enum Op<'a> {
    Start,
    Finish,
    Step(&'a str),
}

/// Classify one reflog message, across the forms git has written.
///
/// Modern git writes `rebase (pick): …`; the interactive one writes
/// `rebase -i (pick): …`; the retired `am` backend wrote `rebase: checkout …`
/// and `rebase finished: returning to …`. All three are read, because a
/// repository's reflog spans whatever versions of git have touched it.
fn classify(message: &str) -> Option<Op<'_>> {
    if !message.starts_with("rebase") {
        return None;
    }
    let head = message.split_once(": ").map(|(h, _)| h).unwrap_or(message);
    if let Some(op) = head.split_once('(').and_then(|(_, r)| r.split_once(')')) {
        return Some(match op.0 {
            "start" => Op::Start,
            "finish" => Op::Finish,
            other => Op::Step(other),
        });
    }
    // No parentheses: the old backend's two messages.
    if head.contains("finished") {
        return Some(Op::Finish);
    }
    let body = message.split_once(": ").map(|(_, b)| b).unwrap_or("");
    if body.starts_with("checkout ") {
        return Some(Op::Start);
    }
    Some(Op::Step("pick"))
}

fn subject_of(message: &str) -> String {
    message
        .split_once(": ")
        .map(|(_, s)| s.trim().to_string())
        .unwrap_or_default()
}

/// The branch a `finish` message names: `returning to refs/heads/x`. A rebase
/// that ran on a detached HEAD names a sha instead, and this is empty.
fn finish_ref(message: &str) -> String {
    message
        .split_whitespace()
        .find(|w| w.starts_with("refs/"))
        .unwrap_or_default()
        .to_string()
}

/// Reconstruct every rebase in `HEAD`'s reflog.
///
/// A rebase runs on a detached HEAD and only moves the branch at the very end,
/// so the whole replay is in `HEAD`'s log — one `start`, one entry per commit
/// written, one `finish` — while the branch's own log holds a single entry for
/// the entire operation. Reading HEAD's is what makes the individual steps
/// visible; the branch's is consulted only when the finish did not name it.
pub fn find_rebases(logs: &BTreeMap<String, Vec<LogEntry>>) -> Vec<Rebase> {
    let Some(head) = logs.get("HEAD") else {
        return Vec::new();
    };

    let mut runs: Vec<Rebase> = Vec::new();
    let mut current: Option<Rebase> = None;
    for entry in head {
        let Some(op) = classify(&entry.message) else {
            // Anything that is not a rebase entry ends an unfinished run:
            // whatever it was doing, it stopped doing it here.
            runs.extend(current.take());
            continue;
        };
        match op {
            Op::Start => {
                runs.extend(current.take());
                current = Some(Rebase {
                    interactive: entry.message.starts_with("rebase -i"),
                    onto: entry.new.clone(),
                    // The tip HEAD was on when the replay began: the commit
                    // this rebase is about to replace.
                    replaced: entry.old.clone(),
                    started_ts: entry.who.ts,
                    started_at: entry.who.iso8601(),
                    ..Default::default()
                });
            }
            Op::Finish => {
                let mut run = current.take().unwrap_or_default();
                run.refname = finish_ref(&entry.message);
                run.result = entry.new.clone();
                run.finished_ts = entry.who.ts;
                run.finished_at = entry.who.iso8601();
                run.completed = true;
                runs.push(run);
            }
            Op::Step(kind) => {
                if let Some(run) = current.as_mut() {
                    run.produced.push(Step {
                        kind: kind.to_string(),
                        oid: entry.new.clone(),
                        subject: subject_of(&entry.message),
                    });
                }
            }
        }
    }
    runs.extend(current.take());

    // A finish that named no branch: find the ref whose own log recorded this
    // very move. Only a rebase on a detached HEAD should reach here.
    for run in &mut runs {
        if !run.refname.is_empty() || run.result.is_empty() {
            continue;
        }
        if let Some((name, _)) = logs.iter().find(|(name, entries)| {
            name.as_str() != "HEAD"
                && entries
                    .iter()
                    .any(|e| e.new == run.result && classify(&e.message).is_some())
        }) {
            run.refname = name.clone();
        }
    }
    runs
}
