mod support;

use support::TestRepo;

#[test]
fn retro_bundle_contains_repository_state() {
    let repo = TestRepo::new("retro");
    repo.run(&["init"]);
    let retro = repo.run(&["retro", "daily"]);
    assert!(retro.contains("retro_bundle"));
    assert!(retro.contains("platform-provided"));
}
