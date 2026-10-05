//! Opening external links. The caller must have obtained explicit user confirmation first;
//! documents can never trigger this on their own.

use std::process::Command;

/// Whether a URI scheme may be opened at all (document-controlled input is untrusted).
pub fn is_allowed_uri(uri: &str) -> bool {
    let lower = uri.trim().to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        && !lower.chars().any(|c| c.is_control())
        && lower.len() < 2048
}

/// Open a confirmed link with the system handler.
pub fn open_confirmed(uri: &str) -> Result<(), String> {
    if !is_allowed_uri(uri) {
        return Err("This kind of link cannot be opened from a document.".into());
    }
    #[cfg(target_os = "windows")]
    let status = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", uri])
        .status();
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(uri).status();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let status = Command::new("xdg-open").arg(uri).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("The system link handler exited with {s}")),
        Err(e) => Err(format!("Could not launch the system link handler: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_are_allowed() {
        assert!(is_allowed_uri("https://example.com/a?b=c"));
        assert!(is_allowed_uri("mailto:a@b.c"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "launch:cmd.exe",
            "ftp://x",
            "https://a\nb",
            "",
        ] {
            assert!(!is_allowed_uri(bad), "{bad}");
        }
    }
}
