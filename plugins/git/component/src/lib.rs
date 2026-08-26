//! The drsg `git` plugin: `../parser` wrapped for the wasm contract.
//!
//! Thin on purpose. The parser is a plain library whose tests run natively,
//! against repositories the `git` binary builds; this crate only adapts it to
//! the contract's two phases and its `Files` trait to the host interface.
//!
//! ## What this plugin is handed
//!
//! Every other plugin is routed by a file's extension, because that is what
//! its input is. A repository's history is not a file — so this one is chosen
//! by the *shape of the source*: the host, digesting a directory that turns
//! out to be a git repository, hands this plugin a view rooted at the
//! repository's **git directory** and nothing else. `HEAD`, `refs/…`,
//! `logs/…`, `objects/…`: a tighter grant than the working tree every other
//! plugin sees, and all this one has any business reading.
//!
//! It declares no extensions for exactly that reason. A plugin that claimed
//! one would be dispatched at files, and there is no file whose extension
//! means "a repository".

use dr_strange_ext::{Edge, Guest, Input, Manifest, Node, Output, Report, export_plugin, host};
use drsg_git_parser::{Files, History, Options, read_history};

/// Shown beside the name in UIs (`manifest.logo`): a commit graph — a line of
/// commits with one branching off and merging back, which is the thing this
/// plugin makes out of a repository.
const LOGO: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='#f05133' stroke-width='1.8' stroke-linecap='round'><path d='M6 21V3'/><path d='M6 8c0 3 12 1 12 4'/><path d='M18 12c0 3-12 1-12 4'/></g><g fill='#f05133'><circle cx='6' cy='3.5' r='2.2'/><circle cx='6' cy='20.5' r='2.2'/><circle cx='18' cy='12' r='2.2'/></g></svg>";

struct GitPlugin;

/// The host interface, as the parser's `Files` trait. Paths are relative to
/// the git directory the host rooted this call at.
struct HostFiles;

impl Files for HostFiles {
    fn list(&self, suffix: &str) -> Result<Vec<String>, String> {
        host::list(suffix)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        host::read(path)
    }
}

impl Guest for GitPlugin {
    fn describe() -> Manifest {
        Manifest {
            name: "git".into(),
            version: "1".into(),
            // None, deliberately — see the module docs: this plugin is chosen
            // by the source being a repository, not by any file's name.
            extensions: Vec::new(),
            logo: Some(LOGO.into()),
        }
    }

    /// A repository is one input, not a tree of them: there is exactly one
    /// object store, one set of refs and one reflog, and every part of the
    /// answer depends on the rest. So the whole read happens here, in the
    /// single chunk the host sends, and `assemble` only decodes it.
    fn parse(subject: Input, options: Vec<(String, String)>) -> Result<Vec<u8>, String> {
        let opts = Options::from_settings(&options)?;
        match subject {
            Input::Files(_) => {}
            Input::Document(doc) => {
                return Err(format!(
                    "`git` reads a repository, not a document — it was handed `{}`. \
                     The host runs this plugin on a directory that has a git \
                     directory in it.",
                    doc.name
                ));
            }
        }
        let history = read_history(&HostFiles, &opts)?;
        rmp_serde::to_vec(&history).map_err(|e| format!("serializing a partial: {e}"))
    }

    fn assemble(partials: Vec<Vec<u8>>, _options: Vec<(String, String)>) -> Result<Output, String> {
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut skipped = 0u32;
        let mut notes: Vec<String> = Vec::new();

        // In the order given — the host guarantees chunk order. There is
        // normally exactly one partial here; the loop is what keeps that an
        // observation rather than an assumption.
        for bytes in &partials {
            let part: History = rmp_serde::from_slice(bytes)
                .map_err(|e| format!("a partial did not decode: {e}"))?;
            notes.push(summarise(&part));
            nodes.extend(part.nodes.into_iter().map(|n| Node {
                key: n.key,
                label: n.label,
                extra_labels: n.extra_labels,
                properties: serde_json::Value::Object(n.props).to_string(),
            }));
            edges.extend(part.edges.into_iter().map(|e| Edge {
                src: e.src,
                dst: e.dst,
                type_: e.ty,
                properties: serde_json::Value::Object(e.props).to_string(),
            }));
            skipped += part.skipped as u32;
            notes.extend(part.notes);
        }

        let facts = (nodes.len() + edges.len()) as u32;
        Ok(Output {
            nodes,
            edges,
            // History is read, never inferred; there is no residue for a model.
            prose: String::new(),
            report: Report {
                facts,
                prose_chars: 0,
                skipped,
                notes,
            },
        })
    }
}

/// The one line a reader wants first: what this repository turned out to be.
fn summarise(h: &History) -> String {
    let s = &h.summary;
    let mut said = format!(
        "{} commit(s), {} of them merges; {} branch(es)",
        s.commits, s.merges, s.branches
    );
    if s.remote_branches > 0 {
        said += &format!(" and {} remote-tracking", s.remote_branches);
    }
    if s.tags > 0 {
        said += &format!("; {} tag(s)", s.tags);
    }
    if s.rebases > 0 {
        said += &format!("; {} rebase(s)", s.rebases);
    }
    if s.unreachable > 0 {
        said += &format!(
            "; {} commit(s) no branch or tag can still reach, kept because a \
             rewrite left them behind",
            s.unreachable
        );
    }
    if !s.head.is_empty() {
        said += &format!("; HEAD is {}", s.head);
    }
    said
}

export_plugin!(GitPlugin);
