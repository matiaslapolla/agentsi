//! Build plumbing for libghostty-vt.
//!
//! - `AGENTSI_GHOSTTY_VT_LIB` names a library file (`libghostty-vt.so`, `.dylib` or `.a`): link that file and run no Zig.
//! - Otherwise build the Ghostty commit pinned in `ghostty.lock` with the Zig 0.17 from `scripts/zig.sh`.
//!   `AGENTSI_GHOSTTY_SOURCE_DIR` names an extracted source tree, for a build with no network.
//!
//! The source tarball is cached in `~/.cache/agentsi/ghostty/`. The build needs `curl` and `tar` only for the download.

use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Lock {
    commit: String,
    version: String,
    url: String,
    sha256: String,
    root: String,
}

impl Lock {
    fn read(path: &Path) -> Lock {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let field = |name: &str| -> String {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == name)
                .map(|(_, v)| v.trim().to_owned())
                .unwrap_or_else(|| panic!("{} has no `{name}` line", path.display()))
        };
        Lock {
            commit: field("commit"),
            version: field("version"),
            url: field("url"),
            sha256: field("sha256"),
            root: field("root"),
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=ghostty.lock");
    for var in [
        "AGENTSI_GHOSTTY_VT_LIB",
        "AGENTSI_GHOSTTY_SOURCE_DIR",
        "AGENTSI_ZIG",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let lock = Lock::read(&manifest.join("ghostty.lock"));

    if let Some(lib) = env::var_os("AGENTSI_GHOSTTY_VT_LIB") {
        link_file(Path::new(&lib));
        println!(
            "cargo:rustc-env=AGENTSI_GHOSTTY_VT_SOURCE=file {}",
            Path::new(&lib).display()
        );
    } else {
        build_with_zig(&manifest, &lock);
        println!(
            "cargo:rustc-env=AGENTSI_GHOSTTY_VT_SOURCE=zig {}",
            lock.commit
        );
    }
}

/// Links a library file that the user built or installed. The file name must be `lib<name>.<ext>`.
fn link_file(lib: &Path) {
    assert!(
        lib.is_file(),
        "AGENTSI_GHOSTTY_VT_LIB is not a file: {}",
        lib.display()
    );
    println!("cargo:rerun-if-changed={}", lib.display());
    let file = lib.file_name().and_then(|f| f.to_str()).unwrap_or_default();
    let (name, ext) = file
        .strip_prefix("lib")
        .and_then(|rest| rest.rsplit_once('.'))
        .filter(|(_, ext)| matches!(*ext, "so" | "dylib" | "a"))
        .unwrap_or_else(|| {
            panic!(
                "AGENTSI_GHOSTTY_VT_LIB must be named lib<name>.so, .dylib or .a (a versioned name such as \
                 libghostty-vt.so.1 needs an unversioned link next to it), got {file}"
            )
        });
    let dir = lib.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    println!("cargo:rustc-link-search=native={}", dir.display());
    if ext == "a" {
        println!("cargo:rustc-link-lib=static={name}");
    } else {
        println!("cargo:rustc-link-lib=dylib={name}");
        // This reaches the targets of this package only. A program in another crate needs the folder
        // in LD_LIBRARY_PATH, or the library in a standard folder.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    }
}

fn build_with_zig(manifest: &Path, lock: &Lock) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = out.join("ghostty-src");
    match env::var_os("AGENTSI_GHOSTTY_SOURCE_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            assert!(
                dir.join("build.zig").is_file(),
                "AGENTSI_GHOSTTY_SOURCE_DIR has no build.zig: {}",
                dir.display()
            );
            replace_dir(&src);
            run(
                Command::new("cp").arg("-R").arg(&dir).arg(&src),
                "cp the source tree",
            );
        }
        None => extract_pinned(lock, &src),
    }

    let zig = zig_path(manifest);
    let prefix = out.join("zig-out");
    let target = env::var("TARGET").unwrap();
    let version = fs::read_to_string(src.join("VERSION"))
        .map(|v| v.trim().to_owned())
        .unwrap_or_else(|_| lock.version.clone());
    let mut build = Command::new(&zig);
    build
        .current_dir(&src)
        .args([
            "build",
            "-Demit-lib-vt",
            "-Doptimize=ReleaseFast",
            "-Dsimd=true",
        ])
        .arg(format!("-Dtarget={}", zig_target(&target)))
        .arg("-Dcpu=baseline")
        .arg(format!("-Dversion-string={version}"))
        .args(["-Demit-xcframework=false", "-Demit-themes=false"])
        .arg("--prefix")
        .arg(&prefix)
        .arg("--cache-dir")
        .arg(out.join("zig-cache"));
    run(&mut build, "zig build");

    let lib_dir = prefix.join("lib");
    assert!(
        lib_dir.join("libghostty-vt.a").is_file(),
        "zig build did not produce {}",
        lib_dir.join("libghostty-vt.a").display()
    );
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}

/// Downloads (once) and extracts the pinned tarball. The sha256 check is the only trust in the download.
fn extract_pinned(lock: &Lock, dest: &Path) {
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .expect("HOME is not set")
        .join("agentsi")
        .join("ghostty");
    fs::create_dir_all(&cache).unwrap();
    let tarball = cache.join(format!("libghostty-vt-source-{}.tar.gz", lock.commit));
    if !fs::read(&tarball).is_ok_and(|b| sha256(&b) == lock.sha256) {
        let part = tarball.with_extension("part");
        run(
            Command::new("curl")
                .args(["-fsSL", "--retry", "3", "-o"])
                .arg(&part)
                .arg(&lock.url),
            &format!("curl {}", lock.url),
        );
        let got = sha256(&fs::read(&part).unwrap());
        assert_eq!(
            got, lock.sha256,
            "checksum mismatch for {}. Set AGENTSI_GHOSTTY_SOURCE_DIR to a tree of commit {}.",
            lock.url, lock.commit
        );
        fs::rename(&part, &tarball).unwrap();
    }
    replace_dir(dest);
    fs::create_dir_all(dest).unwrap();
    run(
        Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(dest)
            .arg("--strip-components=1"),
        "tar -xzf",
    );
    assert!(
        dest.join("build.zig").is_file(),
        "the tarball has no build.zig under {}",
        lock.root
    );
}

fn replace_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).unwrap();
    }
}

fn zig_path(manifest: &Path) -> PathBuf {
    let script = manifest.join("../../scripts/zig.sh");
    let out = Command::new("sh")
        .arg(&script)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", script.display()));
    assert!(
        out.status.success(),
        "scripts/zig.sh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {what}: {e}"));
    assert!(status.success(), "{what} failed with {status}");
}

fn zig_target(target: &str) -> &'static str {
    match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "x86_64-apple-darwin" => "x86_64-macos",
        "aarch64-apple-darwin" => "aarch64-macos",
        other => panic!("agentsi-vt has no Zig target for `{other}`"),
    }
}
