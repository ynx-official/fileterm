//! OSC sequence handlers: parse OSC 7 (CWD), leave 52/1337 for G3.
//!
//! Phase G-1.8 of `docs/plans/active/gpui-spike.md`.
//!
//! ## OSC 7 — CWD 跟随
//!
//! Format: `OSC 7 ; file://<host><path> ST`
//!
//! Shells emit this on every prompt change (bash via `PROMPT_COMMAND`,
//! zsh via `precmd`, fish natively). The payload is a `file://` URL whose
//! path is the shell's current directory. We extract the path and store
//! it on `TermModel::cwd`; the host part is ignored (CWD sync only cares
//! about the path on the *remote* side — for a local PTY the host is
//! `localhost` anyway, and for a future SSH session the host would be
//! the remote hostname, which doesn't change the path semantics).
//!
//! ## Why not `url::Url`?
//!
//! Adding the `url` crate for a single prefix-strip is overkill — it
//! pulls in `idna`, `percent-encoding`, `form_urlencoded`, and `serde`
//! transitive deps. The OSC 7 payload grammar is tight enough that a
//! hand-rolled parser is both shorter and easier to audit. If we later
//! need percent-decoding (rare but legal: `file://host/a%20b`), we can
//! either inline a small decoder or pull in `percent-encoding` alone.
//!
//! ## Robustness
//!
//! Shells occasionally emit malformed OSC 7 (e.g. missing host, double
//! slash, non-UTF-8 bytes on broken locales). We treat any parse failure
//! as "leave `cwd` untouched" rather than panicking — a stale CWD is
//! better than a crashed terminal.

use std::path::PathBuf;

/// Parse an OSC 7 payload into a CWD path.
///
/// `payload` is the raw bytes after `OSC 7 ;` and before the terminator
/// (`BEL` or `ST`). vte hands us this already split on `;`, so for a
/// well-formed `\x1b]7;file://localhost/tmp\x07` the caller passes
/// `b"file://localhost/tmp"`.
///
/// Returns `Some(path)` if the payload is a valid `file://` URL with a
/// non-empty path; `None` otherwise (caller leaves `cwd` untouched).
///
/// # Examples
///
/// ```
/// # use fileterm_gpui::term::osc::parse_osc7_cwd;
/// assert_eq!(parse_osc7_cwd(b"file://localhost/tmp"), Some("/tmp".into()));
/// assert_eq!(parse_osc7_cwd(b"file:///home/user"), Some("/home/user".into()));
/// assert_eq!(parse_osc7_cwd(b"file://example.com/var/log"), Some("/var/log".into()));
/// assert_eq!(parse_osc7_cwd(b"not-a-url"), None);
/// assert_eq!(parse_osc7_cwd(b"file://localhost"), None); // no path
/// ```
pub fn parse_osc7_cwd(payload: &[u8]) -> Option<PathBuf> {
    // `file://` is 7 bytes. Reject anything shorter up front.
    let rest = payload.strip_prefix(b"file://")?;

    // After `file://`, the grammar is `<host><path>` where `<host>` runs
    // until the next `/` (or end of string if there's no path — which we
    // reject as malformed). On Unix a path always starts with `/`, so we
    // find the first `/` after the host.
    //
    // Examples:
    //   `file://localhost/tmp`       → host=`localhost`, path=`/tmp`
    //   `file:///tmp`                → host=`` (empty, legal), path=`/tmp`
    //   `file://example.com/a/b`     → host=`example.com`, path=`/a/b`
    //   `file://localhost`           → no `/` after host → None
    let path_start = rest.iter().position(|&b| b == b'/')?;
    let path_bytes = &rest[path_start..];

    // Reject empty path (shouldn't happen given the `position` find, but
    // defensive — `file://localhost/` would give path_bytes = b"/" which
    // is valid root).
    if path_bytes.is_empty() {
        return None;
    }

    // OSC 7 payloads are ASCII paths in practice, but shells can emit
    // UTF-8 for non-ASCII directory names. `from_utf8` failure means the
    // shell sent invalid UTF-8 (broken locale) — leave cwd untouched.
    let path_str = std::str::from_utf8(path_bytes).ok()?;

    // `PathBuf::from` does no validation beyond accepting the string; a
    // NUL byte or other weirdness would just become part of the path.
    // That's fine — the file manager will reject it when it tries to stat.
    Some(PathBuf::from(path_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_host_with_path() {
        assert_eq!(
            parse_osc7_cwd(b"file://localhost/tmp"),
            Some(PathBuf::from("/tmp"))
        );
    }

    #[test]
    fn empty_host_triple_slash() {
        // `file:///tmp` is the "localhost omitted" form; some shells emit it.
        assert_eq!(
            parse_osc7_cwd(b"file:///home/user"),
            Some(PathBuf::from("/home/user"))
        );
    }

    #[test]
    fn remote_host_ignored_path_kept() {
        assert_eq!(
            parse_osc7_cwd(b"file://example.com/var/log"),
            Some(PathBuf::from("/var/log"))
        );
    }

    #[test]
    fn nested_path_preserved() {
        assert_eq!(
            parse_osc7_cwd(b"file://host/a/b/c/d"),
            Some(PathBuf::from("/a/b/c/d"))
        );
    }

    #[test]
    fn root_path_accepted() {
        // `file://localhost/` → path is `/` (root). Valid, if unusual.
        assert_eq!(
            parse_osc7_cwd(b"file://localhost/"),
            Some(PathBuf::from("/"))
        );
    }

    #[test]
    fn missing_file_prefix_rejected() {
        assert_eq!(parse_osc7_cwd(b"http://localhost/tmp"), None);
        assert_eq!(parse_osc7_cwd(b"localhost/tmp"), None);
        assert_eq!(parse_osc7_cwd(b"/tmp"), None);
    }

    #[test]
    fn no_path_after_host_rejected() {
        // `file://localhost` with no trailing slash — no path at all.
        assert_eq!(parse_osc7_cwd(b"file://localhost"), None);
        assert_eq!(parse_osc7_cwd(b"file://"), None);
    }

    #[test]
    fn empty_payload_rejected() {
        assert_eq!(parse_osc7_cwd(b""), None);
    }

    #[test]
    fn invalid_utf8_rejected() {
        // 0xFF is not valid UTF-8 start byte.
        assert_eq!(parse_osc7_cwd(b"file://localhost/\xFF"), None);
    }

    #[test]
    fn non_file_scheme_rejected() {
        assert_eq!(parse_osc7_cwd(b"http://localhost/tmp"), None);
        assert_eq!(parse_osc7_cwd(b"ftp://localhost/tmp"), None);
        assert_eq!(parse_osc7_cwd(b"ssh://user@host/tmp"), None);
    }
}
