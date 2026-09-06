//! What a project *declares* it depends on.
//!
//! Every code plugin already mints a node for a foreign package the moment
//! something imports it — the ts parser writes `express` when a file writes
//! `import express`. What none of them can know is that the project declared
//! a dependency on it, at a version, in a build manifest none of them read.
//! Those two halves sat one key apart and never met, so "what version of the
//! thing this file imports" had no answer in the graph.
//!
//! This plugin reads the declaration. The dependency it names is keyed the
//! way the code plugins key an import — a package name, a module path — so
//! the two are **one node**, and the version rides on the `DEPENDS_ON` edge
//! rather than on that node: a version is a fact about *this declaration*,
//! and two manifests in a monorepo may well declare different ones.
//!
//! ## Routed by name, not by extension
//!
//! A manifest is a *filename*. `package.json` is not "every `.json`", and
//! claiming that extension would take every fixture and `tsconfig` in the
//! tree from the reader that handles them. So this plugin declares no
//! extensions at all: the host dispatches a known manifest name to whatever
//! is installed under the name `deps`, the same statement it already makes
//! sending a `.git` directory to `git`. Nothing is guessed — with this
//! plugin absent, those files are read as prose exactly as before.
//!
//! ## What is not claimed
//!
//! Lockfiles. `package-lock.json`, `go.sum` and their kin state a *resolved*
//! graph rather than a declaration: they are enormous, they change on every
//! install, and they answer a different question than the one asked here.
//!
//! `Cargo.toml` stays with the `toml` plugin, which already reads it into
//! tables.

use dr_strange_ext::{
    Input, Manifest, Output, OutputExt, Simple, edge, host, node, output, simple_plugin,
};

/// Shown beside the name in UIs: an original mark evoking a bill of
/// materials, not any ecosystem's trademark.
const LOGO: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><rect x='4' y='2' width='16' height='20' rx='2' fill='#2c5282'/><path d='M8 7h8M8 11h8M8 15h5' stroke='#fff' stroke-width='1.8' stroke-linecap='round'/></svg>";

struct Deps;

impl Simple for Deps {
    fn describe() -> Manifest {
        Manifest {
            name: "deps".into(),
            version: "1".into(),
            // Deliberately none: see the module docs. The host routes the
            // filenames it knows to this plugin by its name.
            extensions: Vec::new(),
            logo: Some(LOGO.into()),
        }
    }

    fn process(subject: Input, _options: &[(String, String)]) -> Result<Output, String> {
        let mut out = output();
        let files = match subject {
            Input::Files(paths) => paths,
            Input::Document(doc) => {
                extract(&doc.name, &String::from_utf8_lossy(&doc.bytes), &mut out);
                return Ok(out.finish());
            }
        };
        for path in files {
            match host::read(&path) {
                Ok(bytes) => extract(&path, &String::from_utf8_lossy(&bytes), &mut out),
                Err(why) => {
                    out.report.skipped += 1;
                    out.note(format!("{path}: {why}"));
                }
            }
        }
        Ok(out.finish())
    }
}

/// One declared dependency: what it is called where it is declared, the
/// version as written, and which set it was declared in.
struct Dep {
    key: String,
    version: String,
    scope: &'static str,
}

/// The basename, lowercased — what the host routed on.
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase()
}

/// One manifest into facts: the file, what it declares, and what it depends
/// on. A file that does not parse still yields its node — counted and named,
/// never fatal, because a repository holds a half-written manifest more
/// often than anyone would like.
fn extract(path: &str, text: &str, out: &mut Output) {
    let base = basename(path);
    let (declares, deps) = match base.as_str() {
        "package.json" => package_json(text),
        "go.mod" => go_mod(text),
        "requirements.txt" => requirements(text),
        "pyproject.toml" => pyproject(text),
        "pom.xml" => pom(text),
        b if b.starts_with("build.gradle") => gradle(text),
        _ => (None, Vec::new()),
    };

    let mut file = node(path, "Manifest").described("path", "where the file was read from", path);
    if let Some(name) = &declares {
        file = file.described(
            "declares",
            "the package this manifest is for, named as its own ecosystem names it",
            name.clone(),
        );
    }
    out.nodes.push(file.build());

    if deps.is_empty() {
        return;
    }
    for dep in &deps {
        // A bare stand-in: for a foreign package the key *is* the fact, and
        // leaving it bare is what lets it merge with the node a code plugin
        // mints for the same package from an import.
        out.nodes
            .push(node(&dep.key, "Package").also("External").build());
        let mut e = edge(path, "DEPENDS_ON", &dep.key).described(
            "scope",
            "the set it was declared in: `runtime`, `dev`, `test` or `optional`",
            dep.scope,
        );
        if !dep.version.is_empty() {
            e = e.described(
                "version",
                "the requirement as written — a range, not a resolved version",
                dep.version.clone(),
            );
        }
        out.edges.push(e.build());
    }
    out.note(format!("{path}: {} declared dependenc(ies)", deps.len()));
}

// ---- npm -----------------------------------------------------------------

/// `package.json`: the name it declares, and every dependency set it lists.
///
/// The key is the package name, which is exactly what the ts parser mints
/// for a bare import specifier — so a declared `express` and an imported
/// `express` are one node.
fn package_json(text: &str) -> (Option<String>, Vec<Dep>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return (None, Vec::new());
    };
    let name = v.get("name").and_then(|n| n.as_str()).map(str::to_string);
    let mut deps = Vec::new();
    for (field, scope) in [
        ("dependencies", "runtime"),
        ("devDependencies", "dev"),
        ("peerDependencies", "runtime"),
        ("optionalDependencies", "optional"),
    ] {
        let Some(map) = v.get(field).and_then(|d| d.as_object()) else {
            continue;
        };
        for (key, ver) in map {
            deps.push(Dep {
                key: key.clone(),
                version: ver.as_str().unwrap_or_default().to_string(),
                scope,
            });
        }
    }
    (name, deps)
}

// ---- go ------------------------------------------------------------------

/// `go.mod`: the module path it declares, and its `require`s — single-line
/// and block form alike. The key is the module path, which is what the go
/// parser mints for an import of another module.
fn go_mod(text: &str) -> (Option<String>, Vec<Dep>) {
    let mut module = None;
    let mut deps = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        if in_require {
            if line == ")" {
                in_require = false;
                continue;
            }
            if let Some(dep) = go_require(line) {
                deps.push(dep);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            module = Some(rest.trim().to_string());
        } else if line == "require (" {
            in_require = true;
        } else if let Some(rest) = line.strip_prefix("require ")
            && let Some(dep) = go_require(rest.trim())
        {
            deps.push(dep);
        }
    }
    (module, deps)
}

fn go_require(line: &str) -> Option<Dep> {
    let mut parts = line.split_whitespace();
    let key = parts.next()?.to_string();
    Some(Dep {
        key,
        version: parts.next().unwrap_or_default().to_string(),
        scope: "runtime",
    })
}

// ---- python --------------------------------------------------------------

/// `requirements.txt`, one requirement per line.
///
/// `-r other.txt` names another file rather than a package: recorded as
/// nothing, because following it is resolution and this reads declarations.
fn requirements(text: &str) -> (Option<String>, Vec<Dep>) {
    let mut deps = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        if let Some(dep) = requirement(line, "runtime") {
            deps.push(dep);
        }
    }
    (None, deps)
}

/// One PEP 508 requirement, as far as the *name* goes: `pillow>=10`,
/// `httpx[http2]==0.27`, `uvicorn ; sys_platform != "win32"`.
fn requirement(spec: &str, scope: &'static str) -> Option<Dep> {
    let spec = spec.split(';').next().unwrap_or(spec).trim();
    let end = spec
        .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .unwrap_or(spec.len());
    let (name, rest) = spec.split_at(end);
    if name.is_empty() {
        return None;
    }
    Some(Dep {
        key: name.to_string(),
        version: rest
            .trim_start_matches(|c: char| c == '[' || c.is_alphanumeric() || c == ']')
            .trim()
            .to_string(),
        scope,
    })
}

/// `pyproject.toml`: PEP 621's `[project]`, and poetry's table where a
/// project uses that instead.
fn pyproject(text: &str) -> (Option<String>, Vec<Dep>) {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return (None, Vec::new());
    };
    let mut deps = Vec::new();
    let project = doc.get("project").and_then(|p| p.as_table());
    let name = project
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);
    if let Some(list) = project
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in list {
            if let Some(spec) = item.as_str()
                && let Some(dep) = requirement(spec, "runtime")
            {
                deps.push(dep);
            }
        }
    }
    if let Some(groups) = project
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (_, item) in groups.iter() {
            for spec in item.as_array().into_iter().flatten() {
                if let Some(spec) = spec.as_str()
                    && let Some(dep) = requirement(spec, "optional")
                {
                    deps.push(dep);
                }
            }
        }
    }
    // Poetry states requirements as a table of name → constraint.
    if let Some(table) = doc
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for (key, item) in table.iter() {
            if key == "python" {
                continue; // the interpreter is not a dependency of this kind
            }
            deps.push(Dep {
                key: key.to_string(),
                version: item.as_str().unwrap_or_default().to_string(),
                scope: "runtime",
            });
        }
    }
    (name, deps)
}

// ---- the JVM -------------------------------------------------------------

/// `pom.xml`: the artifact it declares, and its `<dependencies>`.
///
/// A Maven coordinate is `group:artifact`, which is **not** an import root —
/// `com.google.guava:guava` is imported as `com.google.common.*` — so these
/// edges land on a stand-in that nothing else in the graph will name. That is
/// still worth recording: what a build declares is a fact even where "who
/// imports it" cannot be answered from it.
fn pom(text: &str) -> (Option<String>, Vec<Dep>) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut path: Vec<String> = Vec::new();
    let mut deps: Vec<Dep> = Vec::new();
    let mut declares: Option<String> = None;
    let (mut group, mut artifact, mut version, mut scope) =
        (String::new(), String::new(), String::new(), String::new());
    let mut own = (String::new(), String::new());

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                path.push(String::from_utf8_lossy(e.name().as_ref()).to_string())
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "dependency" && !artifact.is_empty() {
                    deps.push(Dep {
                        key: coordinate(&group, &artifact),
                        version: std::mem::take(&mut version),
                        scope: match scope.as_str() {
                            "test" => "test",
                            "provided" => "dev",
                            _ => "runtime",
                        },
                    });
                    group.clear();
                    artifact.clear();
                    scope.clear();
                }
                path.pop();
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                let in_dep = path.iter().any(|p| p == "dependency");
                match (in_dep, path.last().map(String::as_str)) {
                    (true, Some("groupId")) => group = text,
                    (true, Some("artifactId")) => artifact = text,
                    (true, Some("version")) => version = text,
                    (true, Some("scope")) => scope = text,
                    // The project's own coordinate: only at the top level,
                    // never a parent's or a plugin's.
                    (false, Some("groupId")) if path.len() == 2 => own.0 = text,
                    (false, Some("artifactId")) if path.len() == 2 => own.1 = text,
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if !own.1.is_empty() {
        declares = Some(coordinate(&own.0, &own.1));
    }
    (declares, deps)
}

fn coordinate(group: &str, artifact: &str) -> String {
    if group.is_empty() {
        artifact.to_string()
    } else {
        format!("{group}:{artifact}")
    }
}

/// `build.gradle` and `build.gradle.kts`.
///
/// A gradle build is a *program*, not a manifest: dependencies can be
/// computed, aliased through a version catalog, or added by a plugin. This
/// reads the ordinary written forms — `implementation 'g:a:v'`,
/// `api("g:a:v")` — and nothing else, which the README says plainly rather
/// than implying coverage it does not have.
fn gradle(text: &str) -> (Option<String>, Vec<Dep>) {
    const CONFIGS: &[(&str, &str)] = &[
        ("testImplementation", "test"),
        ("testRuntimeOnly", "test"),
        ("androidTestImplementation", "test"),
        ("implementation", "runtime"),
        ("api", "runtime"),
        ("runtimeOnly", "runtime"),
        ("compileOnly", "dev"),
        ("annotationProcessor", "dev"),
    ];
    let mut deps = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or(line).trim();
        let Some((config, scope)) = CONFIGS.iter().find(|(c, _)| line.starts_with(c)) else {
            continue;
        };
        let rest = line[config.len()..].trim();
        // The coordinate is the quoted string, however it was handed over.
        let Some(spec) = rest.split(['\'', '"']).nth(1).filter(|s| s.contains(':')) else {
            continue;
        };
        let mut parts = spec.split(':');
        let (Some(group), Some(artifact)) = (parts.next(), parts.next()) else {
            continue;
        };
        deps.push(Dep {
            key: coordinate(group, artifact),
            version: parts.next().unwrap_or_default().to_string(),
            scope,
        });
    }
    (None, deps)
}

simple_plugin!(Deps);

#[cfg(test)]
mod tests;
