pub mod app;
pub mod db;
pub mod index;
pub mod scope;
pub mod util;
pub mod workflow;

mod index_state;
mod search_index;

pub(crate) const INDEX_DIRTY_KEY: &str = "index_dirty";
