// Rendering of the version string `khora --version` reports.
//
// Shared by `build.rs` (which stamps it into the binary) and the CLI test
// suite, via `include!` — a build script cannot depend on its own crate, and
// the parsing below has enough shapes to be worth testing directly. Line
// comments, not `//!`: an inner doc comment is illegal anywhere but the top of
// the including file (E0753).

/// Render the reported version from the package version and the output of
/// `git describe --tags --always --long`.
///
/// A build sitting exactly on its release tag reports the bare package version
/// (`0.3.17`). Anything past the tag carries a `+<commits>.g<sha>` marker
/// (`0.3.17+13.g6114093`), so a version read cannot understate what the binary
/// can do — the failure this exists to prevent is an agent inferring "no touch
/// commands" from `khora 0.3.17` on a build that has them.
///
/// `describe` is `None` when git is unavailable or the source is not a
/// checkout (a crates.io or Homebrew tarball build), which reports the bare
/// package version — correct, since those are cut from a tag.
fn build_version(pkg_version: &str, describe: Option<&str>) -> String {
    let describe = describe.map(str::trim).filter(|d| !d.is_empty());
    let Some(describe) = describe else {
        return pkg_version.to_string();
    };
    match parse_describe_long(describe) {
        Some((0, _)) => pkg_version.to_string(),
        Some((commits, sha)) => format!("{pkg_version}+{commits}.{sha}"),
        // `--always`'s fallback: a repo with no tag reachable from HEAD, so
        // there is no commit count to report — the bare sha still dates it.
        None => format!("{pkg_version}+{describe}"),
    }
}

/// Split `--long`'s `<tag>-<commits>-g<sha>` shape.
///
/// Split from the right, because a tag may itself contain `-`
/// (`v1.0.0-rc1-3-gabc1234`).
fn parse_describe_long(describe: &str) -> Option<(u32, &str)> {
    let (rest, sha) = describe.rsplit_once('-')?;
    let hex = sha.strip_prefix('g')?;
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (_tag, commits) = rest.rsplit_once('-')?;
    Some((commits.parse().ok()?, sha))
}
