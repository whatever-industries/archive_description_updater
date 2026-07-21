//! The one baked-in transformation: redump.org -> redump.info.

use regex::Regex;
use std::sync::OnceLock;

/// Matches the `redump.org` domain anywhere it appears - bare, in prose, or
/// inside a URL. The trailing `\b` keeps `redump.organization` intact.
fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(redump)\.(org)\b").unwrap())
}

/// Rewrite every `redump.org` to `redump.info`, preserving the original casing
/// of both halves. Works on plain text and on URLs alike, so
/// `www.redump.org/disc/192839128392` becomes
/// `www.redump.info/disc/192839128392` and an `href="http://redump.org/..."`
/// hotlink is rewritten in place.
pub fn fix(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last = 0;

    for caps in re().captures_iter(input) {
        let whole = caps.get(0).unwrap();

        // Only rewrite when "redump" starts the domain label. This leaves a
        // genuinely different domain such as "notredump.org" untouched, while
        // still matching "www.redump.org" (preceded by a dot).
        let starts_label = input[..whole.start()]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());

        out.push_str(&input[last..whole.start()]);
        if starts_label {
            let name = caps.get(1).unwrap().as_str();
            let tld = caps.get(2).unwrap().as_str();
            out.push_str(name);
            out.push('.');
            // Mirror the casing of the .org we are replacing.
            out.push_str(if tld.chars().all(char::is_uppercase) {
                "INFO"
            } else {
                "info"
            });
        } else {
            out.push_str(whole.as_str());
        }
        last = whole.end();
    }

    out.push_str(&input[last..]);
    out
}

/// True if `fix` would change this text.
pub fn needs_fix(input: &str) -> bool {
    re().is_match(input) && fix(input) != input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_bare_domain_preserving_case() {
        assert_eq!(fix("redump.org"), "redump.info");
        assert_eq!(fix("Redump.org"), "Redump.info");
        assert_eq!(fix("REDUMP.ORG"), "REDUMP.INFO");
    }

    #[test]
    fn rewrites_inside_urls() {
        assert_eq!(
            fix("www.redump.org/disc/192839128392"),
            "www.redump.info/disc/192839128392"
        );
        assert_eq!(
            fix(r#"<a href="http://redump.org/disc/123">Redump.org</a>"#),
            r#"<a href="http://redump.info/disc/123">Redump.info</a>"#
        );
        assert_eq!(
            fix("https://redump.org/disc/192839128392/"),
            "https://redump.info/disc/192839128392/"
        );
    }

    #[test]
    fn rewrites_every_occurrence() {
        assert_eq!(
            fix("see redump.org and Redump.org twice"),
            "see redump.info and Redump.info twice"
        );
    }

    #[test]
    fn leaves_unrelated_text_alone() {
        assert_eq!(fix("nothing to do here"), "nothing to do here");
        assert_eq!(fix("redump.info"), "redump.info");
        // A different domain that merely ends in "redump.org".
        assert_eq!(fix("notredump.org"), "notredump.org");
        // Not the TLD - a longer word starting with "org".
        assert_eq!(fix("redump.organization"), "redump.organization");
    }

    #[test]
    fn needs_fix_agrees_with_fix() {
        assert!(needs_fix("visit redump.org"));
        assert!(!needs_fix("visit redump.info"));
        assert!(!needs_fix("notredump.org"));
        assert!(!needs_fix(""));
    }

    #[test]
    fn preserves_surrounding_markup_exactly() {
        let desc = "<b>Source:</b> redump.org<br /><i>note</i>";
        assert_eq!(fix(desc), "<b>Source:</b> redump.info<br /><i>note</i>");
    }
}
