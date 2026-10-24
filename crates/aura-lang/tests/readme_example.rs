//! The example at the top of both READMEs runs, and says what it produces.
//!
//! It is the first code a visitor reads. Two claims are checked here: the
//! output shown is exactly what the manifest produces, run as the README's
//! command line runs it; and the sizes quoted for the larger example the
//! README links to are that example's sizes.

use std::path::Path;
use std::process::Command;

// Only `run` is called; `main` is the example's own entry point.
#[allow(dead_code)]
#[path = "../examples/readme_embed.rs"]
mod readme_embed;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(name: &str) -> String {
    std::fs::read_to_string(root().join(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .replace("\r\n", "\n")
}

/// The body of the first fence that opens with `opening`, after `from`.
fn fence<'a>(text: &'a str, from: usize, opening: &str) -> (&'a str, usize) {
    let start = from + text[from..].find(opening).expect("fence") + opening.len();
    let end = start + text[start..].find("```").expect("closing fence");
    (&text[start..end], end)
}

fn numbers_in(text: &str) -> Vec<usize> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|w| w.parse().ok())
        .collect()
}

fn eval_yaml(dir: &Path, file: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_aura"))
        .current_dir(dir)
        .args(["eval", file, "--format", "yaml"])
        .output()
        .expect("run aura");
    assert!(
        out.status.success(),
        "{file}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn both_readmes_show_what_their_example_produces() {
    for name in ["README.md", "README.ru.md"] {
        let text = read(name);
        let at = text.find("```ruby\n# deploy.aura").expect("the example");
        let (manifest, after) = fence(&text, at, "```ruby\n");
        let (console, _) = fence(&text, after, "```console\n");

        let dir = std::env::temp_dir().join(format!("aura-readme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("deploy.aura"), manifest).expect("write");
        let produced = eval_yaml(&dir, "deploy.aura");
        let _ = std::fs::remove_dir_all(&dir);

        // The whole output is shown, line for line, after the command line.
        let shown: Vec<&str> = console.lines().skip(1).collect();
        let actual: Vec<&str> = produced.trim_end().lines().collect();
        assert_eq!(shown, actual, "{name}: the output shown");
    }
}

#[test]
fn the_larger_example_is_the_size_both_readmes_say() {
    let dir = root().join("examples/k8s_services");
    let yaml = read("examples/k8s_services/expected.yaml");
    let deployments = yaml.matches("kind: Deployment").count();
    let services = yaml.matches("kind: Service\n").count();
    for name in ["README.md", "README.ru.md"] {
        let text = read(name);
        let at = text.find("](examples/k8s_services/)").expect("the link");
        let start = text[..at].rfind("**").expect("the claim");
        let claim = &text[start..at];
        assert_eq!(
            numbers_in(claim),
            [deployments, services, yaml.lines().count()],
            "{name}: {claim}"
        );
    }
    // The file the numbers come from is the example's real output; the
    // conformance test pins it too, this only keeps the claim honest alone.
    assert_eq!(eval_yaml(&dir, "services.aura").trim_end(), yaml.trim_end());
}

/// The Rust shown under "Inside a Rust application" is the example file, line
/// for line, and the example runs and returns what the README says.
#[test]
fn the_embedding_example_is_the_one_that_runs() {
    let file = read("crates/aura-lang/examples/readme_embed.rs");
    let code = file
        .split("// README: start\n")
        .nth(1)
        .and_then(|rest| rest.split("// README: end").next())
        .expect("markers");
    for name in ["README.md", "README.ru.md"] {
        let text = read(name);
        assert!(
            text.contains(&format!("```rust\n{code}```")),
            "{name} does not show examples/readme_embed.rs as it is"
        );
    }
    assert_eq!(readme_embed::run().expect("runs"), 60.0);
}
