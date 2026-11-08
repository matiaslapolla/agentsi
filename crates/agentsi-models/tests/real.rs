//! One download from the real Hugging Face. Run it by hand:
//! `cargo test -p agentsi-models --test real -- --ignored --nocapture`

use std::sync::atomic::AtomicBool;

use agentsi_models::{Catalog, Downloader, State, Store};

#[test]
#[ignore = "uses the network"]
fn downloads_the_smallest_catalog_file() {
    let catalog = Catalog::builtin();
    let mut model = catalog.get("parakeet-tdt-v3-int8").unwrap().clone();
    model.files.retain(|f| f.name == "nemo128.onnx");
    assert_eq!(model.files.len(), 1);
    let root = std::env::temp_dir().join(format!("agentsi-models-real-{}", std::process::id()));
    let store = Store::new(&root);
    let receipt = Downloader::hugging_face()
        .download(&store, &model, &AtomicBool::new(false), |e| {
            eprintln!("{e:?}")
        })
        .unwrap();
    eprintln!("{receipt:?}");
    assert_eq!(store.state(&model), State::Ready);
    store.delete(&model).unwrap();
    assert_eq!(store.state(&model), State::NotDownloaded);
    assert!(!store.file_path(&model, "nemo128.onnx").exists());
    let _ = std::fs::remove_dir_all(&root);
}
