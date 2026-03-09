use std::process::Command;
use std::sync::LazyLock;

static HELP_OUTPUT: LazyLock<String> = LazyLock::new(|| {
    let bin = env!("CARGO_BIN_EXE_stdio-to-ws");
    let output = Command::new(bin)
        .arg("--help")
        .output()
        .expect("failed to run stdio-to-ws --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}{stderr}")
});

/// Goal: Verify --help output includes description text
/// Feature: CLI help text
#[test]
fn test_help_contains_description() {
    assert!(
        HELP_OUTPUT.contains("Bridge stdio"),
        "Expected help output to contain 'Bridge stdio', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --port
/// Feature: CLI help text
#[test]
fn test_help_contains_port() {
    assert!(
        HELP_OUTPUT.contains("--port"),
        "Expected help output to contain '--port', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --quiet
/// Feature: CLI help text
#[test]
fn test_help_contains_quiet() {
    assert!(
        HELP_OUTPUT.contains("--quiet"),
        "Expected help output to contain '--quiet', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --bind
/// Feature: CLI help text
#[test]
fn test_help_contains_bind() {
    assert!(
        HELP_OUTPUT.contains("--bind"),
        "Expected help output to contain '--bind', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --persist
/// Feature: CLI help text
#[test]
fn test_help_contains_persist() {
    assert!(
        HELP_OUTPUT.contains("--persist"),
        "Expected help output to contain '--persist', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --grace-period
/// Feature: CLI help text
#[test]
fn test_help_contains_grace_period() {
    assert!(
        HELP_OUTPUT.contains("--grace-period"),
        "Expected help output to contain '--grace-period', got:\n{}", *HELP_OUTPUT
    );
}

/// Goal: Verify --help output includes --framing
/// Feature: CLI help text
#[test]
fn test_help_contains_framing() {
    assert!(
        HELP_OUTPUT.contains("--framing"),
        "Expected help output to contain '--framing', got:\n{}", *HELP_OUTPUT
    );
}
