//! Model catalog and downloads.
//!
//! - [`Catalog`] lists the models that the app can download (`models.toml`).
//! - [`Store`] knows the model folders, the receipts and the lock.
//! - [`Downloader`] fetches the files of one model with resume and a hash check.
//!
//! The download code follows Hex (`src/net/download.rs`, MIT). See the root `NOTICE`.

mod catalog;
mod download;
mod error;
mod store;

pub use catalog::{Catalog, Kind, Model, ModelFile};
pub use download::{Downloader, Event, HUGGING_FACE};
pub use error::Error;
pub use store::{Receipt, ReceiptFile, State, Store, default_root};
