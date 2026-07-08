mod support;

use support::TestRepo;

fn warning_codes(result: &serde_json::Value) -> Vec<String> {
    result["warnings"]
        .as_array()
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(|warning| warning["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn save_warns_on_missing_tags_and_relative_dates() {
    let repo = TestRepo::new("lint-warnings");
    repo.run(&["init"]);

    let output = repo.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "deadline_note",
        "--content",
        "最近要在下週前完成部署",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("save json");
    assert_eq!(result["status"], "saved");
    let codes = warning_codes(&result);
    assert!(codes.contains(&"no_tags".to_string()), "codes: {codes:?}");
    assert!(
        codes.contains(&"relative_date_language".to_string()),
        "codes: {codes:?}"
    );
}

#[test]
fn save_warns_on_vague_name_and_long_content() {
    let repo = TestRepo::new("lint-vague");
    repo.run(&["init"]);

    let long = "詳細內容 ".repeat(300);
    let output = repo.run(&[
        "save",
        "--type",
        "project",
        "--name",
        "note",
        "--tags",
        "[\"project:example/app\"]",
        "--content",
        &long,
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("save json");
    let codes = warning_codes(&result);
    assert!(
        codes.contains(&"vague_name".to_string()),
        "codes: {codes:?}"
    );
    assert!(
        codes.contains(&"content_long".to_string()),
        "codes: {codes:?}"
    );
}

#[test]
fn clean_save_has_no_warnings() {
    let repo = TestRepo::new("lint-clean");
    repo.run(&["init"]);

    let output = repo.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "pr_review_size",
        "--tags",
        "[\"style:review\",\"decision:pr-size\"]",
        "--content",
        "送出 PR 時，拆成小於 400 行的單位。原因：使用者 2026-07-05 要求逐個 review。",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("save json");
    assert_eq!(result["status"], "saved");
    assert!(result.get("warnings").is_none(), "result: {result}");
}
