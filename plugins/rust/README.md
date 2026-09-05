# plugin: rust

English · [简体中文](README_CN.md)

Parses Rust source into graph facts. Manifest `rust@2`, claims `.rs`.
Built on [syn](https://crates.io/crates/syn) — the parser the macro
ecosystem itself runs on — parse-only: no type inference, no macro
expansion, which is the point. `@2` is the **fact-format version** (the
shape of the facts changed once, from the in-database prototype); it is
independent of the release tag.

## Layout

```
parser/     drsg-rust-parser — the language logic, a plain library, 37 native tests
component/  drsg-plugin-rust — the wasm wrapper: Guest impl + rmp-serde partials
```

## Keys

An item's identity is its **module path** — what a Rust programmer calls it
and what a model will recognise:

```
my_crate                              lib.rs (names the crate root, not "lib")
my_crate::api::cache                  a module (file or inline)
my_crate::api::cache::brute_force_search
my_crate::Thing::read                 an inherent method
<my_crate::Thing as core::fmt::Display>::fmt    a trait-impl method — real
                                      qualified-path syntax, the only thing
                                      keeping six `impl From<…>` blocks from
                                      all claiming one key
```

The crate name comes from the nearest `Cargo.toml`'s `[package] name`
(`-` → `_`), read through the host — so a digest rooted at `…/foo/src`
still keys as `foo::…`, and two crates' `api::Thing` never merge.

## Nodes

| Label | Emitted for | Props beyond `doc_comment` / `visibility` / `file` / `line` |
|---|---|---|
| `Module` | each file and each inline `mod` | `path` (crate-root-relative: `src/compute/cache.rs`), `imports` (resolved use-targets, joined) |
| `Function` / `Method` | free fns, impl fns (`Method` iff it takes `self`) | `signature`, `returns`, `receiver`, `local_bindings`, `is_async` (present only when true) |
| `Struct` / `Enum` / `Union` | type declarations | `fields` (described list of `vis name: type`, declaration order) / `variants` (described list, `Unit`, `Lit(i64)`, `A = 1`); `non_exhaustive` when marked |
| `Trait` | trait declarations | its items become nodes reached by `HAS_METHOD` |
| `Const` / `Static` | consts and statics | type under `signature`, initializer under `value` — **as written, never evaluated** (`256 * 1024` stays an expression) |
| `TypeAlias` | `type X = …` | aliased type under `signature` |
| `Macro` | `macro_rules!` definitions | — |
| stand-ins | anything referenced but not declared here | label says what the reference proved (`Function`, `Trait`, `Type`, bare `External` when only a `use` was seen) + extra label `External`; **no props** — for a stand-in the key *is* the fact |

With `include_source = "true"` (from `[plugins.rust]`), each item also
carries `_code`: the source as written, described as retrieval-only — the
`_` prefix keeps it out of embeddings and the schema summary.

## Edges

| Type | Meaning | `line` |
|---|---|---|
| `CONTAINS` | module → item, type → variant | declaration site |
| `HAS_METHOD` | trait/type → its methods | method's line |
| `CALLS` | function → what it calls | **call site** |
| `IMPLEMENTS` | type → trait (`impl` blocks); `From<i64>` rides the edge as an `impl` prop rather than minting a second `From` node | the `impl` keyword |
| `IMPORTS` | module → what its `use` statements name (with `as_written` when aliased) | the `use` statement |
| `INSTANTIATES` | function → a type it **builds**, with `variant` on the edge when it built one of an enum's. Construction is not a call: `Ok(v)`, `Mine::A(v)`, `Meters(1.0)` and `Widget { .. }` are all spelled like calls or literals and all name a *type*, so none of them mints a `Function` node for a constructor that is no item | construction site |
| `INVOKES` | module → an item-position macro invocation, `arguments` described on the edge — a **marked blind spot**: nothing expands macros, so the items they define are absent, but where they are defined is findable | invocation site |
| `REFERENCES` | function → a function it **passes as a value** rather than calls | the argument |

## Resolution — the certainty line

- A call written as a **path** (`fs::read(…)`, `Vec::new()`) is expanded
  against the file's own `use` list and binds exactly; paths nothing here
  declares become external stand-ins (that is what "this crate uses that"
  needs).
- A **bare name** binds by scope proximity; a name with two equally-close
  candidates is **ambiguous — counted, not guessed**.
- A **method call** (`.read()`) names no path; it resolves as far as the
  receiver's type can be **read off the body**. Type arguments carry through
  every binding a body writes: a parameter or `let` annotation, a field's
  declared type, a constructor path (`Vec::new()`), a declared return, a
  `type` alias with its parameters filled in, a constant, and chains of
  those (`self.items.iter().map(…)`) through a table of what std's own
  methods return — so `for n in &v`, `if let Some(n) = …`, `match` arms on
  the tree's own enums, `let (a, b) = …`, `Node { key, .. }` and the
  parameters of `v.iter().map(|n| …)` are typed, `?`/`.unwrap()` reach a
  `Result<T>`'s `T`, a closure whose body is a chain says what `map`
  yields, and `collect::<Vec<_>>()` is a `Vec`. A **channel constructor** is the pair
  it returns — `let (tx, rx) = mpsc::channel()` types both halves, so
  `tx.send(v)` and `rx.recv()` land on `mpsc::Sender::send` and
  `mpsc::Receiver::recv` instead of the ledger; keyed by the constructor's
  own module (`std::sync::mpsc`, `tokio::sync::mpsc`, `crossbeam::channel`),
  so it is the same node an annotated `let rx: mpsc::Receiver<T>` resolves
  to rather than a second spelling of one type. A type this tree declares
  lands on its own method; `T: Tr`, `impl Tr` and `dyn Tr` land on the
  trait's; a type the tree does not declare — std's, a dependency's — lands
  on an external stand-in keyed by the type that **declares** the method —
  `Vec::push`, `str::trim`, `Clone::clone`, and `Iterator::collect` for every
  concrete iterator (`Range`, `Lines`, `Chars`) that answers through the
  trait — which is all that is known about it. A
  receiver whose type nothing states — an unbounded generic, the result of
  a foreign function no table knows — is **counted, never guessed**, and
  the ledger edge says which hop stopped it.
- **Re-exports** (`pub use`, including `pub(crate) use`) create the facade
  paths later references resolve through.
- A key seen twice is nearly always two `#[cfg]` alternatives of one item —
  settled here (first wins) and counted, not treated as a collision.

Every count lands in the report notes, so a thin graph explains itself:
unresolved method calls, external calls, ambiguous names, unexpanded macro
invocations.

## Options (`[plugins.rust]` in drsg.toml)

| Key | Effect |
|---|---|
| `include_source = "true"` | attach `_code` to items |

## Build & test

```console
$ cd parser    && cargo test          # native, no wasm toolchain
$ DRSG_EVAL_ROOT=/path/to/a/tree cargo test -- --ignored eval_resolution --nocapture
                                      # how calls resolve over a real tree, and what leads the ledger
$ just rust-plugin                    # → component/target/wasm32-wasip2/release/drsg_plugin_rust.wasm
$ drsg plugin install …/drsg_plugin_rust.wasm
```

Partials cross the phase boundary as **MessagePack** (`rmp-serde`): binary
because the partials for a large tree are megabytes, self-describing because
the facts carry `serde_json::Value` properties — the partial format is the
plugin's own business; the host never looks inside.

## Known limits

Macro-generated items are absent (marked by `INVOKES`); trait-method calls
on generic receivers are method calls, hence counted; a chain through a std
method whose return the parser has no fact for (`map.get(k)?.m()`) stops
there; `#[cfg]` selection is not evaluated — both arms' items exist,
duplicates counted.
