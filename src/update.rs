//! Update notice: is a newer release of CANoxide on GitHub?
//!
//! The only network traffic of the program: one HTTPS GET of
//! `https://api.github.com/repos/MechAkh/CANoxide/releases/latest` with the
//! headers `User-Agent: CANoxide/<version>` and
//! `Accept: application/vnd.github+json`. Nothing is downloaded or run: the
//! UI shows the version and opens the release page in the browser on a
//! click. No UI here; the GUI runs `fetch_latest` in a worker thread (the
//! CLI never calls it).

use serde_json::Value;
use std::sync::Arc;

/// The host and the path of the request (constants: no other host is asked).
pub const API_HOST: &str = "api.github.com";
pub const API_PATH: &str = "/repos/MechAkh/CANoxide/releases/latest";
/// The release page used when the answer names no page of this repository.
pub const RELEASES_PAGE: &str = "https://github.com/MechAkh/CANoxide/releases";
/// An `html_url` of the answer is opened only when it starts with this.
pub const RELEASE_URL_PREFIX: &str = "https://github.com/MechAkh/CANoxide/releases/";
/// The longest answer that is read; a longer one is an error.
pub const MAX_ANSWER: usize = 256 * 1024;
/// Resolve, connect, send and receive timeout of the request.
pub const TIMEOUT_MS: i32 = 5_000;
/// The automatic check at start runs when the last successful one is older.
pub const CHECK_INTERVAL_S: u64 = 24 * 3600;
/// The release name is cut to this many characters.
pub const MAX_NAME_CHARS: usize = 80;

/// The latest release as GitHub reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// `tag_name`, for example `v0.6.0`.
    pub tag: String,
    pub html_url: String,
    /// `name` (empty when GitHub has none).
    pub name: String,
    /// `published_at`, for example `2026-11-20T10:00:00Z` (may be empty).
    pub published_at: String,
    pub draft: bool,
    pub prerelease: bool,
}

impl Release {
    /// The version of the tag without the `v` (`0.6.0`), or the tag itself
    /// when it does not parse.
    pub fn version(&self) -> String {
        match parse_version(&self.tag) {
            Some([a, b, c]) => format!("{a}.{b}.{c}"),
            None => self.tag.clone(),
        }
    }

    /// A published release (no draft, no prerelease) newer than `current`.
    pub fn is_update_for(&self, current: &str) -> bool {
        !self.draft && !self.prerelease && is_newer(current, &self.tag)
    }

    /// The page that the button opens (see `release_page`).
    pub fn page(&self) -> &str {
        release_page(&self.html_url)
    }

    /// The release name for the UI: control characters removed, at most
    /// `MAX_NAME_CHARS` characters.
    pub fn display_name(&self) -> String {
        let clean: String = self.name.chars().filter(|c| !c.is_control()).collect();
        let clean = clean.trim();
        if clean.chars().count() > MAX_NAME_CHARS {
            let mut s: String = clean.chars().take(MAX_NAME_CHARS - 1).collect();
            s.push('…');
            s
        } else {
            clean.to_string()
        }
    }

    /// The release date `YYYY-MM-DD` of `published_at`, or "-".
    pub fn display_date(&self) -> String {
        let d = self.published_at.get(..10).unwrap_or("");
        let b = d.as_bytes();
        let ok = b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
        if ok {
            d.to_string()
        } else {
            "-".to_string()
        }
    }
}

/// The function that asks GitHub (`fetch_latest` in the app; the tests
/// inject their own, so no test makes a request).
pub type Fetcher = Arc<dyn Fn() -> Result<Release, String> + Send + Sync>;

/// The real fetcher (`fetch_latest`).
pub fn default_fetcher() -> Fetcher {
    Arc::new(fetch_latest)
}

/// Read the GitHub answer of `releases/latest`. `tag_name` and `html_url`
/// are required; `name`, `published_at`, `draft` and `prerelease` may be
/// missing or null (empty / false).
pub fn parse_latest(json: &str) -> Result<Release, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Update check: the answer is not JSON ({e})"))?;
    let obj = v.as_object().ok_or_else(|| "Update check: the answer is not a JSON object".to_string())?;
    let text = |k: &str| obj.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |k: &str| obj.get(k).and_then(Value::as_bool).unwrap_or(false);
    let tag = text("tag_name").ok_or_else(|| "Update check: the answer has no tag_name".to_string())?;
    let html_url = text("html_url").ok_or_else(|| "Update check: the answer has no html_url".to_string())?;
    Ok(Release { tag, html_url, name: text("name").unwrap_or_default(), published_at: text("published_at").unwrap_or_default(), draft: flag("draft"), prerelease: flag("prerelease") })
}

/// `X.Y.Z` or `vX.Y.Z` (decimal parts only) as numbers.
pub fn parse_version(s: &str) -> Option<[u64; 3]> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let mut out = [0u64; 3];
    let mut parts = s.split('.');
    for o in &mut out {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// True when `tag` is a higher version than `current` (part by part);
/// anything that does not parse is not newer.
pub fn is_newer(current: &str, tag: &str) -> bool {
    match (parse_version(current), parse_version(tag)) {
        (Some(c), Some(t)) => t > c,
        _ => false,
    }
}

/// `html_url` when it is a page under the releases of this repository
/// (`RELEASE_URL_PREFIX` and then only URL path characters), else the fixed
/// releases page.
pub fn release_page(html_url: &str) -> &str {
    match html_url.strip_prefix(RELEASE_URL_PREFIX) {
        Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~/%+".contains(&b)) => html_url,
        _ => RELEASES_PAGE,
    }
}

/// The time now in seconds since 1970-01-01 UTC.
pub fn now_utc() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The automatic check is due: no successful check yet, the last one is at
/// least `CHECK_INTERVAL_S` old, or it lies in the future (the clock went
/// back).
pub fn check_due(last: Option<u64>, now: u64) -> bool {
    match last {
        None => true,
        Some(t) => now < t || now - t >= CHECK_INTERVAL_S,
    }
}

/// The short text of an HTTP status other than 200.
pub fn status_text(code: u32) -> String {
    match code {
        403 | 429 => "Update check: GitHub rate limit reached; try again later".to_string(),
        404 => "Update check: no release found on GitHub".to_string(),
        300..=399 => format!("Update check: GitHub answered with a redirect (HTTP {code}); not followed"),
        _ => format!("Update check: GitHub answered HTTP {code}"),
    }
}

/// The GET of the latest release (blocking; run it in a worker thread).
#[cfg(windows)]
pub fn fetch_latest() -> Result<Release, String> {
    let body = winhttp::get(API_HOST, API_PATH)?;
    let text = std::str::from_utf8(&body).map_err(|_| "Update check: the answer is not UTF-8".to_string())?;
    parse_latest(text)
}

#[cfg(not(windows))]
pub fn fetch_latest() -> Result<Release, String> {
    Err("Update check: only on Windows".to_string())
}

/// The HTTPS GET with WinHTTP (system proxy, certificate faults are errors,
/// no redirects, no cookies, no automatic logon).
#[cfg(windows)]
#[allow(unsafe_code)]
mod winhttp {
    use super::{status_text, MAX_ANSWER, TIMEOUT_MS};
    use core::ffi::c_void;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::*;

    /// A WinHTTP handle, closed when dropped (on every path).
    struct Handle(*mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a handle that WinHTTP returned, closed once.
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The short text of the last WinHTTP error of `what`.
    fn last_error(what: &str) -> String {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        let t = match e {
            ERROR_WINHTTP_TIMEOUT => "timed out".to_string(),
            ERROR_WINHTTP_NAME_NOT_RESOLVED => "api.github.com not found (no network?)".to_string(),
            ERROR_WINHTTP_CANNOT_CONNECT | ERROR_WINHTTP_CONNECTION_ERROR => "no connection to api.github.com".to_string(),
            ERROR_WINHTTP_SECURE_FAILURE | ERROR_WINHTTP_SECURE_INVALID_CA | ERROR_WINHTTP_SECURE_CERT_CN_INVALID => "TLS certificate not accepted".to_string(),
            _ => format!("{what} failed (WinHTTP error {e})"),
        };
        format!("Update check: {t}")
    }

    fn set_u32(h: &Handle, option: u32, value: u32) -> Result<(), String> {
        // SAFETY: `value` lives for the call; the size is that of a u32.
        let ok = unsafe { WinHttpSetOption(h.0, option, (&value as *const u32).cast(), 4) };
        if ok == 0 {
            Err(last_error("WinHttpSetOption"))
        } else {
            Ok(())
        }
    }

    pub(super) fn get(host: &str, path: &str) -> Result<Vec<u8>, String> {
        let agent = wide(&format!("CANoxide/{}", crate::VERSION));
        let host_w = wide(host);
        let path_w = wide(path);
        let verb = wide("GET");
        let headers = wide("Accept: application/vnd.github+json\r\n");
        // SAFETY (all calls below): the wide strings live until the end of
        // the function, the handles are checked for null before use and
        // closed by `Handle::drop` (request, connection, session).
        let session = Handle(unsafe { WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, core::ptr::null(), core::ptr::null(), 0) });
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        if unsafe { WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS) } == 0 {
            return Err(last_error("WinHttpSetTimeouts"));
        }
        set_u32(&session, WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER)?;
        let connect = Handle(unsafe { WinHttpConnect(session.0, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0) });
        if connect.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let request = Handle(unsafe { WinHttpOpenRequest(connect.0, verb.as_ptr(), path_w.as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null(), WINHTTP_FLAG_SECURE) });
        if request.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        set_u32(&request, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES)?;
        set_u32(&request, WINHTTP_OPTION_AUTOLOGON_POLICY, WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH)?;
        // The header length without the terminating zero.
        let hlen = (headers.len() - 1) as u32;
        if unsafe { WinHttpSendRequest(request.0, headers.as_ptr(), hlen, core::ptr::null(), 0, 0, 0) } == 0 {
            return Err(last_error("WinHttpSendRequest"));
        }
        if unsafe { WinHttpReceiveResponse(request.0, core::ptr::null_mut()) } == 0 {
            return Err(last_error("WinHttpReceiveResponse"));
        }
        let mut code: u32 = 0;
        let mut size: u32 = 4;
        let ok = unsafe { WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, core::ptr::null(), (&mut code as *mut u32).cast(), &mut size, core::ptr::null_mut()) };
        if ok == 0 {
            return Err(last_error("WinHttpQueryHeaders"));
        }
        if code != 200 {
            return Err(status_text(code));
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut n: u32 = 0;
            if unsafe { WinHttpReadData(request.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) } == 0 {
                return Err(last_error("WinHttpReadData"));
            }
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n as usize]);
            if body.len() > MAX_ANSWER {
                return Err("Update check: the answer is longer than 256 KiB".to_string());
            }
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{
        "url": "https://api.github.com/repos/MechAkh/CANoxide/releases/1",
        "html_url": "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0",
        "tag_name": "v0.6.0",
        "name": "CANoxide 0.6.0",
        "draft": false,
        "prerelease": false,
        "published_at": "2026-11-20T10:00:00Z",
        "assets": [],
        "body": "Release notes are not read."
    }"#;

    #[test]
    fn parse_github_answer() {
        let r = parse_latest(ANSWER).unwrap();
        assert_eq!(r.tag, "v0.6.0");
        assert_eq!(r.version(), "0.6.0");
        assert_eq!(r.html_url, "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0");
        assert_eq!(r.name, "CANoxide 0.6.0");
        assert_eq!(r.display_date(), "2026-11-20");
        assert!(!r.draft && !r.prerelease);
        assert!(r.is_update_for("0.5.2"));
        assert!(!r.is_update_for("0.6.0"));
    }

    #[test]
    fn parse_draft_and_prerelease_are_no_update() {
        let d = parse_latest(&ANSWER.replace(r#""draft": false"#, r#""draft": true"#)).unwrap();
        assert!(d.draft && !d.is_update_for("0.5.2"));
        let p = parse_latest(&ANSWER.replace(r#""prerelease": false"#, r#""prerelease": true"#)).unwrap();
        assert!(p.prerelease && !p.is_update_for("0.5.2"));
    }

    #[test]
    fn parse_missing_fields_and_garbage() {
        let no_tag = parse_latest(r#"{"html_url": "https://github.com/MechAkh/CANoxide/releases/tag/v1.0.0"}"#);
        assert!(no_tag.unwrap_err().contains("tag_name"));
        let no_url = parse_latest(r#"{"tag_name": "v1.0.0"}"#);
        assert!(no_url.unwrap_err().contains("html_url"));
        // Optional fields missing or null.
        let r = parse_latest(r#"{"tag_name": "v1.0.0", "html_url": "x", "name": null}"#).unwrap();
        assert_eq!((r.name.as_str(), r.published_at.as_str(), r.draft, r.prerelease), ("", "", false, false));
        assert_eq!(r.display_date(), "-");
        // A tag that is not a string.
        assert!(parse_latest(r#"{"tag_name": 5, "html_url": "x"}"#).is_err());
        assert!(parse_latest("not json").unwrap_err().contains("not JSON"));
        assert!(parse_latest("[1, 2]").unwrap_err().contains("not a JSON object"));
        assert!(parse_latest("").is_err());
        // The rate limit answer of GitHub.
        assert!(parse_latest(r#"{"message": "API rate limit exceeded"}"#).is_err());
    }

    #[test]
    fn version_compare() {
        assert!(!is_newer("0.5.2", "v0.5.2"), "equal");
        assert!(!is_newer("0.5.2", "0.5.2"), "equal without v");
        assert!(!is_newer("0.5.2", "v0.5.1"), "older patch");
        assert!(!is_newer("0.5.2", "v0.4.9"), "older minor");
        assert!(!is_newer("1.0.0", "v0.9.9"), "older major");
        assert!(is_newer("0.5.2", "v0.5.3"), "newer patch");
        assert!(is_newer("0.5.2", "v0.6.0"), "newer minor");
        assert!(is_newer("0.5.2", "v1.0.0"), "newer major");
        assert!(is_newer("0.5.2", "V0.5.3"), "capital V");
        assert!(is_newer("0.5.2", "0.5.3"), "no prefix");
        assert!(is_newer("v0.5.2", "v0.5.10"), "two-digit patch, not text order");
        assert!(is_newer("0.9.0", "0.10.0"), "two-digit minor");
        assert!(!is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("9.0.0", "10.0.0"), "two-digit major");
        for bad in ["", "v", "latest", "v0.6", "v0.6.0.1", "v0.6.x", "v0.6.0-rc1", "v 0.6.0", "v0..6", "v-1.0.0", "v+1.0.0", "v99999999999999999999.0.0"] {
            assert!(!is_newer("0.5.2", bad), "{bad:?} is not newer");
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert!(!is_newer("garbage", "v9.9.9"), "a current version that does not parse");
        assert_eq!(parse_version(" v1.2.3 "), Some([1, 2, 3]));
    }

    #[test]
    fn release_url_check() {
        let ok = "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0";
        assert_eq!(release_page(ok), ok);
        for bad in [
            "",
            "https://github.com/MechAkh/CANoxide/releases/",
            "https://github.com/MechAkh/CANoxide/releases",
            "http://github.com/MechAkh/CANoxide/releases/tag/v0.6.0",
            "https://github.com/Other/CANoxide/releases/tag/v0.6.0",
            "https://github.com.evil.example/MechAkh/CANoxide/releases/tag/v0.6.0",
            "https://evil.example/https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0",
            "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0\" onclick=",
            "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0 x",
            "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0?x=1",
            "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0#x",
            "https://github.com/MechAkh/CANoxide/releases/tag/v0.6.0@evil.example",
            "https://github.com/MechAkh/CANoxide/releases/tag/v\u{e9}",
        ] {
            assert_eq!(release_page(bad), RELEASES_PAGE, "{bad:?}");
        }
        let r = Release { tag: "v1.0.0".into(), html_url: "javascript:alert(1)".into(), name: String::new(), published_at: String::new(), draft: false, prerelease: false };
        assert_eq!(r.page(), RELEASES_PAGE);
    }

    #[test]
    fn name_is_cut_and_cleaned() {
        let mut r = parse_latest(ANSWER).unwrap();
        r.name = format!("A\u{7}B\n{}", "x".repeat(200));
        let n = r.display_name();
        assert!(n.starts_with("AB"));
        assert_eq!(n.chars().count(), MAX_NAME_CHARS);
        assert!(n.ends_with('…'));
        r.name = "  Short  ".into();
        assert_eq!(r.display_name(), "Short");
    }

    #[test]
    fn due_and_status_texts() {
        let now = 1_800_000_000;
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - 3600), now));
        assert!(!check_due(Some(now - CHECK_INTERVAL_S + 1), now));
        assert!(check_due(Some(now - CHECK_INTERVAL_S), now));
        assert!(check_due(Some(now + 3600), now), "a check in the future (clock went back)");
        assert!(status_text(403).contains("rate limit"));
        assert!(status_text(429).contains("rate limit"));
        assert!(status_text(404).contains("no release"));
        assert!(status_text(302).contains("not followed"));
        assert!(status_text(500).contains("HTTP 500"));
    }
}
