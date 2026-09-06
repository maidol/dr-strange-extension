//! The plugin's own tests, running natively.

use super::*;
use dr_strange_ext::Value;

fn run(name: &str, text: &str) -> Output {
    let mut out = output();
    extract(name, text, &mut out);
    out
}

/// The dependencies a manifest declared, as `(key, version, scope)`.
fn declared(o: &Output) -> Vec<(String, String, String)> {
    o.edges
        .iter()
        .filter(|e| e.type_ == "DEPENDS_ON")
        .map(|e| {
            let props: Value = e.properties.parse().expect("edge properties are JSON");
            let read = |k: &str| {
                props
                    .get(k)
                    .and_then(|v| v.get("$value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            (e.dst.clone(), read("version"), read("scope"))
        })
        .collect()
}

fn declares(o: &Output, path: &str) -> Option<String> {
    let n = o.nodes.iter().find(|n| n.key == path)?;
    let props: Value = n.properties.parse().ok()?;
    Some(props.get("declares")?.get("$value")?.as_str()?.to_string())
}

/// The key is the package name, which is exactly what the ts parser mints
/// for a bare import specifier — that identity is the whole point.
#[test]
fn package_json_declares_a_name_and_its_dependency_sets() {
    let out = run(
        "package.json",
        r#"{
          "name": "acme",
          "version": "1.2.3",
          "dependencies": { "express": "^4.18.2" },
          "devDependencies": { "vitest": "~1.0.0" },
          "optionalDependencies": { "fsevents": "*" }
        }"#,
    );
    assert_eq!(declares(&out, "package.json").as_deref(), Some("acme"));
    let mut got = declared(&out);
    got.sort();
    assert_eq!(
        got,
        vec![
            ("express".into(), "^4.18.2".into(), "runtime".into()),
            ("fsevents".into(), "*".into(), "optional".into()),
            ("vitest".into(), "~1.0.0".into(), "dev".into()),
        ]
    );
    // The dependency node is a stand-in: for a foreign package the key is
    // the fact, and the `External` label is the assertion that lets it meet
    // the node an import already minted instead of colliding with it.
    let dep = out.nodes.iter().find(|n| n.key == "express").unwrap();
    let props: Value = dep.properties.parse().unwrap();
    assert!(props.as_object().is_none_or(|o| o.is_empty()), "{props:?}");
    assert!(dep.extra_labels.iter().any(|l| l == "External"));
}

/// Both `require` forms, and a module path keyed the way the go parser keys
/// an import of another module.
#[test]
fn go_mod_reads_the_module_and_both_require_forms() {
    let out = run(
        "go.mod",
        "module example.com/demo\n\
         \n\
         go 1.22\n\
         \n\
         require github.com/lone/pkg v1.0.0\n\
         \n\
         require (\n\
         \tgithub.com/a/b v1.2.3\n\
         \tgithub.com/c/d v0.1.0 // indirect\n\
         )\n",
    );
    assert_eq!(
        declares(&out, "go.mod").as_deref(),
        Some("example.com/demo")
    );
    let mut got: Vec<String> = declared(&out)
        .into_iter()
        .map(|(k, v, _)| format!("{k}@{v}"))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "github.com/a/b@v1.2.3",
            "github.com/c/d@v0.1.0",
            "github.com/lone/pkg@v1.0.0",
        ]
    );
    // `go 1.22` is the language version, not a dependency.
    assert!(!declared(&out).iter().any(|(k, _, _)| k == "go"));
}

/// A requirement is a name plus a constraint, and the name is all this is
/// certain of. `-r` names a file, not a package.
#[test]
fn requirements_reads_names_through_their_constraints() {
    let out = run(
        "requirements.txt",
        "# a comment\n\
         pillow>=10.0\n\
         httpx[http2]==0.27.0\n\
         uvicorn ; sys_platform != \"win32\"\n\
         -r other.txt\n\
         \n",
    );
    let names: Vec<String> = declared(&out).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(names, vec!["pillow", "httpx", "uvicorn"]);
    assert!(!names.iter().any(|n| n.contains("other")));
}

#[test]
fn pyproject_reads_pep621_and_poetry_alike() {
    let out = run(
        "pyproject.toml",
        "[project]\n\
         name = \"acme\"\n\
         dependencies = [\"httpx>=0.27\", \"pydantic\"]\n\
         \n\
         [project.optional-dependencies]\n\
         dev = [\"pytest\"]\n",
    );
    assert_eq!(declares(&out, "pyproject.toml").as_deref(), Some("acme"));
    let mut got: Vec<(String, String)> =
        declared(&out).into_iter().map(|(k, _, s)| (k, s)).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("httpx".to_string(), "runtime".to_string()),
            ("pydantic".to_string(), "runtime".to_string()),
            ("pytest".to_string(), "optional".to_string()),
        ]
    );

    let poetry = run(
        "pyproject.toml",
        "[tool.poetry]\nname = \"p\"\n\n[tool.poetry.dependencies]\npython = \"^3.11\"\nhttpx = \"^0.27\"\n",
    );
    let names: Vec<String> = declared(&poetry).into_iter().map(|(k, _, _)| k).collect();
    assert_eq!(
        names,
        vec!["httpx"],
        "the interpreter is not this kind of dependency"
    );
}

/// A Maven coordinate is `group:artifact` — not an import root, which the
/// README says outright. The project's own coordinate comes from the top
/// level, never from a `<parent>` or a plugin's.
#[test]
fn pom_reads_coordinates_and_scopes() {
    let out = run(
        "pom.xml",
        r#"<project>
             <groupId>com.acme</groupId>
             <artifactId>service</artifactId>
             <dependencies>
               <dependency>
                 <groupId>com.google.guava</groupId>
                 <artifactId>guava</artifactId>
                 <version>33.0.0-jre</version>
               </dependency>
               <dependency>
                 <groupId>org.junit.jupiter</groupId>
                 <artifactId>junit-jupiter</artifactId>
                 <version>5.10.0</version>
                 <scope>test</scope>
               </dependency>
             </dependencies>
           </project>"#,
    );
    assert_eq!(
        declares(&out, "pom.xml").as_deref(),
        Some("com.acme:service")
    );
    let mut got = declared(&out);
    got.sort();
    assert_eq!(
        got,
        vec![
            (
                "com.google.guava:guava".into(),
                "33.0.0-jre".into(),
                "runtime".into()
            ),
            (
                "org.junit.jupiter:junit-jupiter".into(),
                "5.10.0".into(),
                "test".into()
            ),
        ]
    );
}

/// A gradle build is a program. The written forms are read; anything
/// computed is not, and the README does not pretend otherwise.
#[test]
fn gradle_reads_the_written_forms_and_leaves_the_computed_alone() {
    let out = run(
        "build.gradle.kts",
        "dependencies {\n\
         \timplementation(\"com.google.guava:guava:33.0.0-jre\")\n\
         \tapi 'org.apache.commons:commons-lang3:3.14.0'\n\
         \ttestImplementation(\"org.junit.jupiter:junit-jupiter:5.10.0\")\n\
         \timplementation(libs.something)\n\
         \t// implementation(\"commented:out:1.0\")\n\
         }\n",
    );
    let mut got = declared(&out);
    got.sort();
    assert_eq!(
        got,
        vec![
            (
                "com.google.guava:guava".into(),
                "33.0.0-jre".into(),
                "runtime".into()
            ),
            (
                "org.apache.commons:commons-lang3".into(),
                "3.14.0".into(),
                "runtime".into()
            ),
            (
                "org.junit.jupiter:junit-jupiter".into(),
                "5.10.0".into(),
                "test".into()
            ),
        ]
    );
    assert!(
        !got.iter().any(|(k, _, _)| k.contains("commented")),
        "a commented-out line is not a declaration"
    );
}

/// A repository holds a half-written manifest more often than anyone would
/// like. It still yields its node, and nothing is fatal.
#[test]
fn a_manifest_that_does_not_parse_still_yields_its_file() {
    for (name, text) in [
        ("package.json", "{ this is not json"),
        ("pyproject.toml", "[project"),
        ("pom.xml", "<project><dependencies>"),
    ] {
        let out = run(name, text);
        assert!(
            out.nodes.iter().any(|n| n.key == name),
            "{name} lost its file node"
        );
        assert!(declared(&out).is_empty(), "{name} invented dependencies");
    }
}

/// Every manifest in a monorepo is its own node, keyed by path, and the
/// packages two of them share are one node.
#[test]
fn each_manifest_is_its_own_node() {
    let mut out = output();
    extract(
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{"express":"^4"}}"#,
        &mut out,
    );
    extract(
        "apps/api/package.json",
        r#"{"name":"api","dependencies":{"express":"^5"}}"#,
        &mut out,
    );
    assert_eq!(
        declares(&out, "apps/web/package.json").as_deref(),
        Some("web")
    );
    assert_eq!(
        declares(&out, "apps/api/package.json").as_deref(),
        Some("api")
    );
    // Two declarations, two edges, two versions — the version is a fact about
    // the declaration, which is why it is not on the package.
    let versions: Vec<String> = declared(&out).into_iter().map(|(_, v, _)| v).collect();
    assert_eq!(versions, vec!["^4", "^5"]);
}
