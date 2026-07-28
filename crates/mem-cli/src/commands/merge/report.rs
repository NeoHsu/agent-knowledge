#[derive(Debug, Default, serde::Serialize)]
pub(super) struct DurableEventMergeReport {
    pub(super) ambiguities_imported: usize,
    pub(super) ambiguities_identical: usize,
    pub(super) workflow_runs_imported: usize,
    pub(super) workflow_runs_identical: usize,
    pub(super) workflow_runs_unresolved: usize,
    pub(super) changelog_imported: usize,
    pub(super) changelog_identical: usize,
    pub(super) changelog_unresolved: usize,
    pub(super) semantic_revisions_imported: usize,
    pub(super) semantic_revisions_identical: usize,
    pub(super) semantic_revisions_unresolved: usize,
}
