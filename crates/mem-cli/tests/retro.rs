use crate::support::TestRepo;

#[test]
fn retro_bundle_contains_repository_state() {
    let repo = TestRepo::new("retro");
    repo.run(&["init"]);
    let retro = repo.run(&["retro", "daily"]);
    assert!(retro.contains("retro_bundle"));
    assert!(retro.contains("platform-provided"));
}

#[test]
fn weekly_retro_includes_graph_curation_context() {
    let repo = TestRepo::new("retro-graph");
    repo.run(&["init"]);
    let retro: serde_json::Value =
        serde_json::from_str(&repo.run(&["retro", "weekly"])).expect("weekly retro json");
    assert!(retro["pending_graph_edges"]["edges"].is_array());
    assert!(
        retro["instructions"]
            .as_array()
            .expect("instructions")
            .iter()
            .any(|instruction| instruction
                .as_str()
                .unwrap_or_default()
                .contains("pending_graph_edges"))
    );
}
