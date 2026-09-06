# plugin: deps

English · [简体中文](README_CN.md)

Reads **build manifests** into graph facts: what a project declares it
depends on, and at what version. Manifest `deps@1`, claims **no file
extension** — see [Dispatch](#dispatch). Pure Rust: `serde_json`,
`toml_edit` and `quick-xml`, plus two line formats.

Every code plugin already mints a node for a foreign package the moment
something imports it — the ts parser writes `express` when a file writes
`import express`. None of them can know that the project *declared* a
dependency on it, at a version, in a file none of them read. Those halves sat
one key apart and never met, so "what version of the thing this file imports"
had no answer.

This plugin reads the declaration, and keys the dependency the way the code
plugins key an import. The two are then **one node**.

## Dispatch

Routing everywhere else is by extension, because that is what the input is. A
manifest is a **filename**: `package.json` is not "every `.json`", and
claiming that extension would take every fixture and `tsconfig` in the tree
from the reader that handles them.

So this plugin declares no extensions, and the **host** routes the manifest
names it knows to whatever is installed under the name `deps` — the same
statement it already makes sending a `.git` directory to `git`. The host
keeps a *list* of such plugins, so a repository running two build systems is
read by both.

Nothing is guessed: with this plugin absent, those files are read as prose
exactly as before, and the report says which extensions no plugin claimed.

| File | Read from it | Dependency key |
|---|---|---|
| `package.json` | `name`; `dependencies`, `devDependencies`, `peerDependencies`, `optionalDependencies` | the package name — what the ts parser mints for a bare specifier |
| `go.mod` | `module`; `require`, single-line and block | the module path — what the go parser mints for an import |
| `requirements.txt` | one requirement per line | the distribution name |
| `pyproject.toml` | PEP 621 `[project]`, and poetry's table | the distribution name |
| `pom.xml` | the project's own coordinate; `<dependencies>` | `group:artifact` |
| `build.gradle`, `build.gradle.kts` | `implementation` / `api` / `testImplementation` / … lines | `group:artifact` |

## Nodes and edges

| | Meaning |
|---|---|
| `Manifest` | one per file, keyed by path, with `declares` — the package this manifest is for. A monorepo's many `package.json` are many nodes |
| `Package` + `External` | the dependency, keyed as its ecosystem names it. `External` is an assertion that the key names something outside the tree, and two handlers asserting the same foreign key agree rather than collide — which is what makes this node and the one an import already minted **the same node** |
| `DEPENDS_ON` | manifest → dependency, with `version` as written and `scope` (`runtime`, `dev`, `test`, `optional`) on the edge |

The version rides on the **edge**, not the package: a version is a fact about
*this declaration*, and two manifests in one repository may well declare
different ones.

## Where the join holds, and where it does not

- **npm and go join cleanly.** A declared `express` and an imported `express`
  are the same key, and so the same node; likewise a Go module path.
- **Python joins by luck.** A distribution is not an import root —
  `requirements.txt` says `pillow` where the code says `PIL`. Where they
  differ, the declaration lands on a node no import will ever name.
- **Maven and Gradle do not join at all.** A coordinate is
  `com.google.guava:guava`; the java parser keys by package
  (`com.google.common.…`). Nothing maps one to the other without a guess, and
  this family does not guess. The dependency is recorded anyway — what a
  build declares is worth knowing even where "who imports it" cannot be
  answered from it — and its stand-in simply stands alone.

## Known limits

**A gradle build is a program, not a manifest.** Dependencies can be
computed, aliased through a version catalog, or contributed by a plugin. This
reads the ordinary written forms and nothing else: `implementation
'g:a:v'`, `api("g:a:v")`. `implementation(libs.something)` names a catalog
entry this cannot resolve and is skipped rather than guessed at.

**Lockfiles are not read.** `package-lock.json`, `go.sum` and their kin state
a *resolved* graph rather than a declaration; they are enormous, they change
on every install, and they answer a different question.

**`Cargo.toml` stays with the `toml` plugin**, which already reads it into
tables. `pyproject.toml` does *not*: the filename routes here, because
dependency semantics are something a generic TOML reader cannot know. A plane
digested before this plugin existed holds `Table` nodes for that file and
will hold a `Manifest` after.

**A manifest that does not parse still yields its file node**, counted and
named. A repository holds a half-written manifest more often than anyone
would like, and refusing the whole ingest over one would be absurd.

## Catalog entry

The release workflow writes `version`, `url` and `sha256` into `catalog.json`
for a plugin that is already listed there, and refuses to invent a row for one
that is not — `claims` and `min_drsg` are judgements, not mechanics. This
plugin's row is not in the catalog yet, because neither field can be stated
honestly before its first release: the hash is the published artifact's, and
the host that dispatches a manifest filename to `deps` is unreleased. Add it
by hand alongside the `deps-v1.0.0` tag:

| field | value |
|---|---|
| `claims` | `build manifests` — the extension column is prose here, as it is for `git`, because neither plugin is reached by extension |
| `min_drsg` | `2.7.0` — the release that routes a manifest filename to this plugin. Under an older host nothing would ever be handed to it |

Until that row exists the artifact still installs by URL; it is only
`drsg plugin install deps`, which resolves a name through the catalog, that
needs it.

## Build & test

```console
$ cd plugins/deps && cargo test    # 8 tests, native
$ just deps-plugin                 # → target/wasm32-wasip2/release/drsg_plugin_deps.wasm
$ drsg plugin install …/drsg_plugin_deps.wasm
```
