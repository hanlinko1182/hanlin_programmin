use std::process::Command;

fn hanlin_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hanlin"))
}

#[test]
fn test_smoke_examples() {
    let examples = vec![
        "examples/hello.hl",
        "examples/arithmetic.hl",
        "examples/functions.hl",
        "examples/fibonacci.hl",
        "examples/arrays.hl",
        "examples/objects.hl",
        "examples/parse_words.hl",
    ];

    for ex in examples {
        let output = hanlin_bin()
            .arg(ex)
            .output()
            .expect("Failed to execute hanlin binary");
        assert!(
            output.status.success(),
            "Example {} failed: {}",
            ex,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn test_cli_flags() {
    // --emit-tokens
    let output = hanlin_bin()
        .arg("examples/hello.hl")
        .arg("--emit-tokens")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Token Stream"),
        "missing token stream header"
    );
    assert!(stdout.contains("Print"), "missing Print token");

    // --emit-ast
    let output = hanlin_bin()
        .arg("examples/hello.hl")
        .arg("--emit-ast")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Abstract Syntax Tree"),
        "missing AST header"
    );
    assert!(stdout.contains("Print"), "missing Print node");

    // --emit-c
    let output = hanlin_bin()
        .arg("examples/hello.hl")
        .arg("--emit-c")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("#include"), "missing #include");
    assert!(stdout.contains("int main"), "missing int main");
    assert!(stdout.contains("Hello, World!"), "missing Hello, World!");
}

#[test]
fn test_error_exits() {
    // missing source file
    let output = hanlin_bin().arg("nonexistent_file.hl").output().unwrap();
    assert!(
        !output.status.success(),
        "expected failure for nonexistent file"
    );

    // no arguments
    let output = hanlin_bin().output().unwrap();
    assert!(
        !output.status.success(),
        "expected failure for no arguments"
    );
}

#[test]
fn test_invalid_syntax_error_output() {
    let output = hanlin_bin()
        .arg("tests/inputs/invalid_syntax.hl")
        .output()
        .unwrap();
    assert!(!output.status.success(), "expected parsing error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2:1"),
        "error should mention line and column"
    );
    assert!(
        stderr.contains("[ParseError]"),
        "error should say ParseError"
    );
}
