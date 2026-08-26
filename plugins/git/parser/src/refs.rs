//! Refs: what a branch or a tag currently points at.
//!
//! Two places again, because git keeps them in two: a **loose** ref is a file
//! under `refs/` holding an oid, and `git gc` folds refs into a single
//! `packed-refs` file. A loose ref wins where both exist — that is precisely
//! how an update becomes visible before the next pack.

use crate::Files;

/// Where `HEAD` is.
#[derive(Debug, Clone, PartialEq)]
pub enum Head {
    /// The ordinary case: `ref: refs/heads/main`.
    OnBranch(String),
    /// Checked out at a commit with no branch.
    Detached(String),
    /// A repository with `git init` and nothing since — `HEAD` names a branch
    /// that does not exist yet.
    Unborn(String),
}

/// One ref as the files record it, before anything is peeled or classified.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRef {
    /// The full refname, e.g. `refs/heads/main`.
    pub name: String,
    /// What it points at — a commit, or a tag object for an annotated tag.
    pub oid: String,
    /// The commit an annotated tag peels to, when `packed-refs` recorded it.
    /// Absent means it has to be read from the tag object itself.
    pub peeled: Option<String>,
}

/// Every ref, loose and packed, sorted by name with loose winning.
pub fn read_refs(files: &dyn Files, listing: &[String]) -> Vec<RawRef> {
    let mut refs: std::collections::BTreeMap<String, RawRef> = std::collections::BTreeMap::new();

    if let Ok(bytes) = files.read("packed-refs") {
        for r in parse_packed_refs(&String::from_utf8_lossy(&bytes)) {
            refs.insert(r.name.clone(), r);
        }
    }

    for path in listing.iter().filter(|p| p.starts_with("refs/")) {
        let Ok(bytes) = files.read(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let value = text.trim();
        // `refs/remotes/origin/HEAD` is symbolic: it names another ref rather
        // than an object. Nothing here follows it — the ref it names is in
        // this same listing, and recording a pointer to a pointer as if it
        // were a commit would be a lie about what it is.
        if value.starts_with("ref:") || !is_oid(value) {
            continue;
        }
        refs.insert(
            path.clone(),
            RawRef {
                name: path.clone(),
                oid: value.to_string(),
                peeled: None,
            },
        );
    }

    refs.into_values().collect()
}

/// `packed-refs`: an oid and a name per line, and a `^oid` line after an
/// annotated tag giving the commit it peels to.
pub(crate) fn parse_packed_refs(text: &str) -> Vec<RawRef> {
    let mut out: Vec<RawRef> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(peeled) = line.strip_prefix('^') {
            if let Some(last) = out.last_mut() {
                last.peeled = Some(peeled.trim().to_string());
            }
            continue;
        }
        let Some((oid, name)) = line.split_once(' ') else {
            continue;
        };
        if !is_oid(oid) {
            continue;
        }
        out.push(RawRef {
            name: name.trim().to_string(),
            oid: oid.to_string(),
            peeled: None,
        });
    }
    out
}

/// Read `HEAD`, and say which of the three things it is.
pub fn read_head(files: &dyn Files, refs: &[RawRef]) -> Option<Head> {
    let bytes = files.read("HEAD").ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let value = text.trim();
    match value.strip_prefix("ref:") {
        Some(name) => {
            let name = name.trim().to_string();
            Some(if refs.iter().any(|r| r.name == name) {
                Head::OnBranch(name)
            } else {
                Head::Unborn(name)
            })
        }
        None if is_oid(value) => Some(Head::Detached(value.to_string())),
        None => None,
    }
}

pub fn is_oid(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}
