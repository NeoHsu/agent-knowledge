mod support;

use support::TestRepo;

fn parse_error(output: &str) -> serde_json::Value {
    serde_json::from_str(output.trim())
        .unwrap_or_else(|error| panic!("expected one JSON error object, got {output:?}: {error}"))
}

#[test]
fn json_errors_wrap_runtime_failures() {
    let repo = TestRepo::new("json-runtime-error");

    let output = repo.run_fail(&["--json-errors", "query", "missing"]);
    let error = parse_error(&output);

    assert_eq!(error["status"], "error");
    assert_eq!(error["contract_version"], 1);
    assert_eq!(error["code"], "command_failed");
    assert_eq!(error["exit_code"], 1);
    assert!(error["message"]
        .as_str()
        .expect("message")
        .contains("memory store not found"));
}

#[test]
fn json_errors_wrap_clap_parse_failures() {
    let repo = TestRepo::new("json-parse-error");

    let output = repo.run_fail(&["--json-errors", "not-a-command"]);
    let error = parse_error(&output);

    assert_eq!(error["status"], "error");
    assert_eq!(error["contract_version"], 1);
    assert_eq!(error["code"], "cli_parse_error");
    assert_eq!(error["exit_code"], 2);
    assert!(error["message"]
        .as_str()
        .expect("message")
        .contains("unrecognized subcommand"));
}

#[test]
fn json_errors_redact_secret_like_parse_input() {
    let repo = TestRepo::new("json-secret-error");
    let secret = ["ghp_", "abcdefghijklmnop1234567890"].concat();

    let output = repo.run_fail(&["--json-errors", &secret]);
    let error = parse_error(&output);

    assert_eq!(error["code"], "cli_parse_error");
    assert!(!output.contains(&secret));
    assert!(error["message"]
        .as_str()
        .expect("message")
        .contains("[REDACTED]"));
}

#[test]
fn default_errors_remain_human_readable() {
    let repo = TestRepo::new("human-error");

    let output = repo.run_fail(&["not-a-command"]);

    assert!(serde_json::from_str::<serde_json::Value>(output.trim()).is_err());
    assert!(output.contains("unrecognized subcommand"));
}

#[test]
fn json_errors_does_not_change_help_success_output() {
    let repo = TestRepo::new("json-help");

    let output = repo.run(&["--json-errors", "--help"]);

    assert!(output.contains("Portable agent memory CLI"));
    assert!(output.contains("--json-errors"));
}
