use std::time::Duration;

use super::*;

pub(super) const BUNDLE_FORMAT_VERSION: i64 = 2;

const MAX_BUNDLE_ENTRIES: usize = 10_000;
const MAX_BUNDLE_FILE_BYTES: u64 = 1_073_741_824;
const MAX_BUNDLE_TOTAL_BYTES: u64 = 4_294_967_296;
const MAX_BUNDLE_PATH_BYTES: usize = 4_096;
const SNAPSHOT_PAGES_PER_STEP: i32 = 256;
const SNAPSHOT_STEP_PAUSE: Duration = Duration::from_millis(5);

mod archive;
mod export;
mod import;
mod install;
mod profile;
mod rollback;
mod validation;

use export::cmd_bundle_export;
use import::{cmd_bundle_import, cmd_bundle_inspect};

pub(crate) fn cmd_bundle(app: &App, command: BundleCommand) -> Result<()> {
    match command {
        BundleCommand::Export(args) => cmd_bundle_export(app, args),
        BundleCommand::Inspect(args) => cmd_bundle_inspect(args),
        BundleCommand::Import(args) => cmd_bundle_import(app, args),
    }
}
