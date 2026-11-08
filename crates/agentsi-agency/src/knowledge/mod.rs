//! The knowledge base: the index of the docs, journal, commits, issues,
//! minutes and memory of a product, and the reads that search it. U19 ports
//! `indexer.ts`, `search.ts`, `repos.ts`, `code-graph.ts` and the data reads of
//! the MCP server. The ranking of the search is in `agentsi-store`.

pub mod code_graph;
pub mod collect;
pub mod exec;
pub mod index;
pub mod memory_text;
pub mod minutes_md;
pub mod query;
pub mod repos;

pub use index::index_product;
