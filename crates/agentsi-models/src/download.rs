//! Resumable downloads into `.partial` files. One ranged request per piece, so a stalled
//! connection fails in bounded time and every retry resumes where the file ends.
//! The piece loop and the 416 and 200 handling follow Hex `src/net/download.rs` (MIT).

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::store::Receipt;
use crate::{Error, Model, ModelFile, State, Store};

pub const HUGGING_FACE: &str = "https://huggingface.co";

const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What a download reports. All sizes are in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Bytes of one file, and of the whole model, on disk so far.
    Progress {
        file: String,
        file_index: usize,
        files_total: usize,
        file_bytes: u64,
        file_size: u64,
        bytes: u64,
        total: u64,
    },
    /// A piece failed. The download tries again from the end of the `.partial` file.
    Retry {
        file: String,
        attempt: u32,
        error: String,
    },
    /// The last byte arrived. The SHA-256 check runs.
    Verifying { file: String },
    /// The file passed the check and is in place.
    FileDone { file: String },
    /// Every file is in place and the receipt is written.
    Done,
}

#[derive(Debug, Clone)]
pub struct Downloader {
    base_url: String,
    user_agent: String,
    piece: u64,
    attempts: u32,
    retry_delay: Duration,
    piece_timeout: Duration,
}

impl Downloader {
    /// Downloads from Hugging Face. HTTPS only.
    pub fn hugging_face() -> Self {
        Self::with_base_url(HUGGING_FACE)
    }

    /// Downloads from another server with the Hugging Face URL layout
    /// (`<base>/<repo>/resolve/<revision>/<path>`). `http://` is allowed here, for tests.
    pub fn with_base_url(base: impl Into<String>) -> Self {
        Self {
            base_url: base.into().trim_end_matches('/').to_owned(),
            user_agent: concat!("agentsi-models/", env!("CARGO_PKG_VERSION")).to_owned(),
            piece: 8 << 20,
            attempts: 3,
            retry_delay: Duration::from_secs(2),
            piece_timeout: Duration::from_secs(300),
        }
    }

    /// Bytes asked for in one request. At least 1.
    pub fn piece_size(mut self, bytes: u64) -> Self {
        self.piece = bytes.max(1);
        self
    }

    /// Failed pieces in a row that end the download. At least 1.
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    pub fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    fn url(&self, model: &Model, file: &ModelFile) -> String {
        format!(
            "{}/{}/resolve/{}/{}",
            self.base_url,
            model.repo,
            model.revision,
            file.repo_path()
        )
    }

    /// Brings `model` to the `Ready` state, and returns its receipt.
    ///
    /// - A `Ready` model returns at once.
    /// - A second download of the same model, here or in another process, fails with `Busy`.
    /// - A `.partial` file continues with an HTTP `Range` request.
    /// - A file with a wrong hash is deleted, and the call fails with `HashMismatch`.
    /// - A file that appears at a target path during the download stays, and the call fails
    ///   with `Conflict`. The `.partial` file stays too.
    /// - `cancel` set to `true` stops the call with `Canceled` and keeps the `.partial` file.
    pub fn download(
        &self,
        store: &Store,
        model: &Model,
        cancel: &AtomicBool,
        mut on_event: impl FnMut(Event),
    ) -> Result<Receipt, Error> {
        let _lock = store.lock(model)?;
        if store.state(model) == State::Ready
            && let Some(receipt) = store.receipt(model)
        {
            return Ok(receipt);
        }
        let dir = store.model_dir(model);
        let total = model.total_size();
        let mut done_before = 0;
        for (index, file) in model.files.iter().enumerate() {
            let target = dir.join(&file.name);
            let partial = store.partial_path(model, file);
            let progress = |file_bytes: u64, on_event: &mut dyn FnMut(Event)| {
                on_event(Event::Progress {
                    file: file.name.clone(),
                    file_index: index,
                    files_total: model.files.len(),
                    file_bytes,
                    file_size: file.size,
                    bytes: done_before + file_bytes,
                    total,
                });
            };
            // A file from an earlier run, or one the owner placed there, counts if its hash fits.
            if target.exists() {
                if fs::metadata(&target)?.len() == file.size && sha256_file(&target)? == file.sha256
                {
                    progress(file.size, &mut on_event);
                    on_event(Event::FileDone {
                        file: file.name.clone(),
                    });
                    done_before += file.size;
                    continue;
                }
                return Err(Error::Conflict {
                    file: file.name.clone(),
                });
            }
            if fs::metadata(&partial).is_ok_and(|m| m.len() > file.size) {
                fs::remove_file(&partial)?;
            }
            self.fetch(
                &self.url(model, file),
                file,
                &partial,
                cancel,
                &mut |step| match step {
                    Step::Bytes(bytes) => progress(bytes, &mut on_event),
                    Step::Retry(attempt, error) => on_event(Event::Retry {
                        file: file.name.clone(),
                        attempt,
                        error,
                    }),
                },
            )?;
            on_event(Event::Verifying {
                file: file.name.clone(),
            });
            let actual = sha256_file(&partial)?;
            if actual != file.sha256 {
                fs::remove_file(&partial)?;
                return Err(Error::HashMismatch {
                    file: file.name.clone(),
                    expected: file.sha256.clone(),
                    actual,
                });
            }
            place(&partial, &target, &file.name)?;
            on_event(Event::FileDone {
                file: file.name.clone(),
            });
            done_before += file.size;
        }
        let receipt = Receipt::for_model(model);
        store.write_receipt(model, &receipt)?;
        on_event(Event::Done);
        Ok(receipt)
    }

    /// Fills `partial` to `file.size` bytes, with retries.
    fn fetch(
        &self,
        url: &str,
        file: &ModelFile,
        partial: &Path,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(Step),
    ) -> Result<(), Error> {
        // Hugging Face redirects to a signed CDN URL. Later pieces go there directly,
        // and back through the original URL when the signed one fails.
        let mut location = url.to_owned();
        let mut failures = 0;
        let mut throttle = Throttle::new();
        loop {
            let have = fs::metadata(partial).map_or(0, |m| m.len());
            emit(Step::Bytes(have));
            if have >= file.size {
                return Ok(());
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Canceled);
            }
            let result = self.piece(
                location != url,
                &location,
                (have, file.size),
                partial,
                cancel,
                &mut |bytes| {
                    if throttle.ready() {
                        emit(Step::Bytes(bytes))
                    }
                },
            );
            match result {
                Ok(next) => {
                    location = next;
                    failures = 0;
                }
                Err(Failure::Fatal(error)) => return Err(error),
                Err(Failure::Retry(message)) => {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Canceled);
                    }
                    failures += 1;
                    if failures >= self.attempts {
                        return Err(Error::Network(message));
                    }
                    emit(Step::Retry(failures, message));
                    location = url.to_owned();
                    std::thread::sleep(self.retry_delay);
                }
            }
        }
    }

    /// One ranged request. Returns the URL for the next piece.
    fn piece(
        &self,
        signed: bool,
        url: &str,
        (have, size): (u64, u64),
        partial: &Path,

        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<String, Failure> {
        let end = size.min(have.saturating_add(self.piece)) - 1;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(self.base_url.starts_with("https://"))
            .max_redirects(10)
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(self.piece_timeout))
            .build()
            .into();
        let mut response = agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .header("Range", format!("bytes={have}-{end}"))
            .call()
            .map_err(|e| match e {
                // A signed CDN link that expired answers 403: retry through the origin.
                ureq::Error::StatusCode(code)
                    if (400..500).contains(&code) && ![408, 429].contains(&code) && !signed =>
                {
                    Failure::Fatal(Error::Network(format!(
                        "the server answered {code} for {}",
                        url_host(url)
                    )))
                }
                other => Failure::Retry(format!("{} could not be reached: {other}", url_host(url))),
            })?;
        let next = ureq::ResponseExt::get_uri(&response).to_string();
        let status = response.status().as_u16();
        let mut out = match status {
            206 => {
                let start = response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("bytes "))
                    .and_then(|v| v.split_once('-'))
                    .and_then(|(start, _)| start.parse::<u64>().ok());
                if start != Some(have) {
                    return Err(Failure::Retry(
                        "the server sent a different part of the file than asked for".into(),
                    ));
                }
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(partial)
                    .map_err(io)?
            }
            // The server ignores ranges: the whole file, from the start.
            200 => OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .open(partial)
                .map_err(io)?,
            other => return Err(Failure::Retry(format!("the server answered {other}"))),
        };
        let mut written = if status == 200 { 0 } else { have };
        let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Fatal(Error::Canceled));
            }
            let read = reader.read(&mut buffer).map_err(|e| {
                Failure::Retry(format!("the connection to {} broke: {e}", url_host(url)))
            })?;
            if read == 0 {
                break;
            }
            if written + read as u64 > size {
                return Err(Failure::Fatal(Error::BadResponse(
                    "the server sent more than the pinned size of the file".into(),
                )));
            }
            out.write_all(&buffer[..read]).map_err(io)?;
            written += read as u64;
            progress(written);
        }
        out.flush().map_err(io)?;
        // A piece that adds nothing, or a 200 that stops short, counts as a failure.
        // Otherwise a server that sends empty bodies would loop for ever.
        let whole = status == 200 && written == size;
        if !whole && written <= have {
            return Err(Failure::Retry("the server sent no new bytes".into()));
        }
        if status == 200 && !whole {
            return Err(Failure::Retry("the server sent a short file".into()));
        }
        Ok(next)
    }
}

enum Step {
    Bytes(u64),
    Retry(u32, String),
}

enum Failure {
    Retry(String),
    Fatal(Error),
}

fn io(e: std::io::Error) -> Failure {
    Failure::Fatal(Error::Io(e))
}

/// The host of a URL, for messages. Never the path or the query: a signed URL holds a token.
fn url_host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split(['/', '?']).next())
        .unwrap_or("the server")
}

struct Throttle(Instant);

impl Throttle {
    fn new() -> Self {
        Self(Instant::now())
    }
    fn ready(&mut self) -> bool {
        if self.0.elapsed() >= PROGRESS_EVERY {
            self.0 = Instant::now();
            true
        } else {
            false
        }
    }
}

/// Moves a checked `.partial` file to `target`, and never replaces a file there: the owner
/// may have put one there while the download ran. A hard link fails when `target` exists.
fn place(partial: &Path, target: &Path, name: &str) -> Result<(), Error> {
    match fs::hard_link(partial, target) {
        Ok(()) => {
            fs::remove_file(partial)?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(Error::Conflict {
            file: name.to_owned(),
        }),
        // A file system with no hard links. The check and the rename leave a short window.
        Err(_) if target.exists() => Err(Error::Conflict {
            file: name.to_owned(),
        }),
        Err(_) => Ok(fs::rename(partial, target)?),
    }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
