use std::fs;
use std::process::Command;

fn hanlin_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hanlin"))
}

fn run_source(name: &str, source: &str) -> std::process::Output {
    let path = std::env::temp_dir().join(format!("hanlin_{name}_{}.hl", std::process::id()));
    fs::write(&path, source).expect("failed to write temporary Hanlin source");
    let output = hanlin_bin()
        .arg(&path)
        .output()
        .expect("failed to run Hanlin");
    let _ = fs::remove_file(path);
    output
}

#[test]
fn compound_assignments_work_for_all_assignable_targets() {
    let output = run_source(
        "compound_assignments",
        r#"
            let x = 10;
            x += 5;
            x -= 3;
            x *= 2;
            x /= 2;
            let arr = [1];
            arr[0] += 4;
            let obj = { count: 2 };
            obj.count *= 3;
            print(x, arr[0], obj.count);
        "#,
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "12 5 6");
}

#[test]
fn floating_modulo_by_zero_returns_runtime_error() {
    let output = run_source("float_modulo_zero", "let result = 5.0 % 0.0;");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Modulo by zero"));
}

#[test]
fn join_without_delimiter_uses_comma() {
    let output = run_source("join_default", "print([1, 2, 3].join());");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1,2,3");
}

#[test]
fn native_else_if_and_null_are_executable() {
    let output = run_source(
        "else_if_null",
        r#"
            let value;
            if (value == null) {
                print("null");
            } else if (true) {
                print("wrong");
            }
        "#,
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "null");
}
