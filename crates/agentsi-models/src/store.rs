use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Model, ModelFile};

const RECEIPT: &str = "receipt.json";
const LOCK: &str = ".lock";
pub(crate) const PARTIAL: &str = ".partial";

/// `<data>/agentsi/models`. `$XDG_DATA_HOME` replaces `~/.local/share` on Linux.
/// On macOS it is `~/Library/Application Support/agentsi/models`.
pub fn default_root() -> Option<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    };
    let data = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else {
        match std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
            Some(x) => PathBuf::from(x),
            None => home()?.join(".local/share"),
        }
    };
    Some(data.join("agentsi/models"))
}

/// Written after the last file of a model passed its hash check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub repo: String,
    pub revision: String,
    pub files: Vec<ReceiptFile>,
    /// Seconds since the Unix epoch.
    pub downloaded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl Receipt {
    pub(crate) fn for_model(model: &Model) -> Self {
        Self {
            id: model.id.clone(),
            repo: model.repo.clone(),
            revision: model.revision.clone(),
            files: model
                .files
                .iter()
                .map(|f| ReceiptFile {
                    name: f.name.clone(),
                    size: f.size,
                    sha256: f.sha256.clone(),
                })
                .collect(),
            downloaded_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        }
    }

    fn matches(&self, model: &Model) -> bool {
        self.id == model.id
            && self.revision == model.revision
            && self.files.len() == model.files.len()
            && model
                .files
                .iter()
                .zip(&self.files)
                .all(|(m, r)| m.name == r.name && m.size == r.size && m.sha256 == r.sha256)
    }
}

/// What the settings screen shows for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotDownloaded,
    /// Some bytes are on disk (finished files and `.partial` files).
    Partial {
        bytes: u64,
    },
    Ready,
}

/// The model folders under one root.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/catalog/<id>`. The `catalog` folder is the app's own. The files that the
    /// Python voice code keeps in `<root>/kokoro/`, `<root>/piper/` and so on stay untouched.
    pub fn model_dir(&self, model: &Model) -> PathBuf {
        self.root.join("catalog").join(&model.id)
    }

    /// The path of a file of `model`, for the speech code to open.
    pub fn file_path(&self, model: &Model, name: &str) -> PathBuf {
        self.model_dir(model).join(name)
    }

    /// Where the bytes of a file arrive until they pass the hash check.
    pub(crate) fn partial_path(&self, model: &Model, file: &ModelFile) -> PathBuf {
        self.model_dir(model)
            .join(format!("{}{PARTIAL}", file.name))
    }

    pub fn receipt(&self, model: &Model) -> Option<Receipt> {
        let text = fs::read_to_string(self.model_dir(model).join(RECEIPT)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// `Ready` needs a receipt for this exact catalog entry and every file at its pinned size.
    /// It does not hash the files. The download did that.
    pub fn state(&self, model: &Model) -> State {
        let dir = self.model_dir(model);
        let len = |p: PathBuf| fs::metadata(p).map(|m| m.len()).ok();
        let ready = self.receipt(model).is_some_and(|r| r.matches(model))
            && model
                .files
                .iter()
                .all(|f| len(dir.join(&f.name)) == Some(f.size));
        if ready {
            return State::Ready;
        }
        let bytes: u64 = model
            .files
            .iter()
            .map(|f| {
                len(dir.join(&f.name))
                    .or_else(|| len(self.partial_path(model, f)))
                    .unwrap_or(0)
            })
            .sum();
        if bytes == 0 {
            State::NotDownloaded
        } else {
            State::Partial { bytes }
        }
    }

    /// Takes the lock of one model. A second caller, in this process or another, gets `Busy`.
    pub(crate) fn lock(&self, model: &Model) -> Result<Lock, Error> {
        let dir = self.model_dir(model);
        fs::create_dir_all(&dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(LOCK))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::Busy(model.id.clone())),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }

    /// Deletes the files of a model that this crate wrote: finished files, `.partial` files and receipt.
    /// A running download of the same model makes it fail with `Busy`.
    pub fn delete(&self, model: &Model) -> Result<(), Error> {
        let dir = self.model_dir(model);
        if !dir.exists() {
            return Ok(());
        }
        let _lock = self.lock(model)?;
        // Only what this crate wrote. A catalog file counts as ours when a receipt lists it, or
        // when it is the pinned file (a download that failed on a later file leaves no receipt).
        // Any other file with that name may be a file of the owner.
        // A `.partial` file and the receipt are always ours.
        let listed: Vec<String> = self
            .receipt(model)
            .map(|r| r.files.into_iter().map(|f| f.name).collect())
            .unwrap_or_default();
        let mut ours = vec![RECEIPT.to_owned(), format!("{RECEIPT}.tmp")];
        for f in &model.files {
            if listed.contains(&f.name) || is_pinned(&dir.join(&f.name), f) {
                ours.push(f.name.clone());
            }
            ours.push(format!("{}{PARTIAL}", f.name));
        }
        for name in ours {
            match fs::remove_file(dir.join(&name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        // The lock file stays: removing it while another caller has it open would let two
        // callers hold two different locks.
        Ok(())
    }

    pub(crate) fn write_receipt(&self, model: &Model, receipt: &Receipt) -> Result<(), Error> {
        let dir = self.model_dir(model);
        let tmp = dir.join(format!("{RECEIPT}.tmp"));
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(receipt).expect("receipt serializes"),
        )?;
        fs::rename(tmp, dir.join(RECEIPT))?;
        Ok(())
    }
}

/// The file at `path` has the pinned size and hash of `file`.
fn is_pinned(path: &Path, file: &ModelFile) -> bool {
    fs::metadata(path).is_ok_and(|m| m.len() == file.size)
        && crate::download::sha256_file(path).is_ok_and(|h| h == file.sha256)
}

/// Held while a download runs. Dropping it releases the lock.
pub(crate) struct Lock {
    _file: File,
}
