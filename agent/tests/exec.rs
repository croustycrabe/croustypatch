//! Tests for `exec`.
use croustypatch_agent::exec::*;

#[tokio::test]
async fn runs_without_shell_and_clean_env() {
    // Shell metacharacters are plain arguments to echo.
    let out = run(&Cmd::new("echo", &["a;b", "$(id)", "&&"])).await.unwrap();
    assert_eq!(out.stdout.trim(), "a;b $(id) &&");
    // The caller's environment does not leak.
    std::env::set_var("CP_SECRET", "x");
    let out = run(&Cmd::new("env", &[])).await.unwrap();
    assert!(!out.stdout.contains("CP_SECRET"));
    assert!(out.stdout.contains("LC_ALL=C"));
}

#[tokio::test]
async fn timeout_and_missing_program() {
    assert!(run(&Cmd::new("sleep", &["5"]).timeout(1)).await.unwrap_err().to_string().contains("timed out"));
    assert!(run(&Cmd::new("definitely-not-a-program", &[])).await.is_err());
    assert!(find_program("../bin/sh").is_none());
}
