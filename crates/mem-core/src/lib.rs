pub mod app;
pub mod config;
pub mod db;
pub mod index;
pub mod scope;
pub mod util;
pub mod workflow;

mod index_state;
mod search_index;
mod search_tokenizer;

pub(crate) const INDEX_DIRTY_KEY: &str = "index_dirty";
