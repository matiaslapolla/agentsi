//! Downloads one catalog model into `<models>/catalog/<id>/`.
//! `cargo run -p agentsi-voice --example fetch -- kokoro`

use agentsi_models::{Catalog, Downloader, Event, Store, default_root};
use std::sync::atomic::AtomicBool;

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "kokoro".into());
    let catalog = Catalog::builtin();
    let model = catalog.get(&id).expect("a catalog id");
    let store = Store::new(default_root().expect("HOME is set"));
    let mut last = 0;
    let receipt = Downloader::hugging_face()
        .download(&store, model, &AtomicBool::new(false), |e| match e {
            Event::Progress { bytes, total, .. } => {
                let pct = bytes * 100 / total.max(1);
                if pct >= last + 5 {
                    last = pct;
                    eprintln!("{pct}%");
                }
            }
            other => eprintln!("{other:?}"),
        })
        .expect("the download ends");
    println!("ready: {}", store.model_dir(model).display());
    println!("{receipt:?}");
}
