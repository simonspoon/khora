//! Stamps the git-derived build version into the binary as `KHORA_BUILD_VERSION`.
//!
//! See `build_version.rs` for the format and why it exists.

use std::path::PathBuf;
use std::process::Command;

include!("build_version.rs");

fn main() {
    let pkg_version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let describe = git(&["describe", "--tags", "--always", "--long"]);

    for path in rerun_paths() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!(
        "cargo:rustc-env=KHORA_BUILD_VERSION={}",
        build_version(&pkg_version, describe.as_deref())
    );
}

/// Run a git command in the package directory, or `None` if git is missing or
/// this is not a checkout. Never fails the build — a tarball build has no git
/// and reports the bare package version.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Files whose change can move `git describe`, so the stamp is not left stale
/// by cargo's caching — a stale marker would reintroduce the very
/// understatement this stamp exists to prevent.
///
/// `HEAD` and `logs/HEAD` live in the *per-worktree* git dir (which differs
/// from the common dir inside a `git worktree`); refs and tags are shared.
/// Only existing paths are emitted: a nonexistent `rerun-if-changed` path
/// forces a rebuild on every invocation.
fn rerun_paths() -> Vec<PathBuf> {
    let git_dir = git(&["rev-parse", "--absolute-git-dir"]).map(PathBuf::from);
    let common_dir = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(PathBuf::from)
        .or_else(|| git_dir.clone());
    let mut paths = Vec::new();
    if let Some(dir) = &git_dir {
        // `logs/HEAD` is appended by commit/checkout/reset/merge/rebase alike,
        // which makes it the single cheapest catch-all for "HEAD moved".
        paths.extend([dir.join("HEAD"), dir.join("logs").join("HEAD")]);
    }
    if let Some(dir) = &common_dir {
        // A newly cut tag changes describe without moving HEAD.
        paths.extend([dir.join("packed-refs"), dir.join("refs").join("tags")]);
    }
    paths.retain(|p| p.exists());
    paths
}
