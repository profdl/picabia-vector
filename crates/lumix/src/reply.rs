//! A `cam.cgi` reply.
//!
//! *Hypothesis* (community notes): the camera answers each command with a small XML document,
//! `<camrply><result>ok</result>…</camrply>`, where `result` is `ok` or an `err_*` code
//! (`err_reject`, `err_busy`, `err_unsuitable_app`, `err_non_support`, …), and `getstate` adds a
//! `<state>` element of leaf values (`batt`, `cammode`, `sdcardstatus`, `rec`, …). The reply is
//! read leniently: every leaf element's text is kept by name, and a reply that isn't XML at all
//! still gives its text for the log.

/// The most of a reply that is read.
pub const MAX_REPLY: usize = 16 * 1024;
/// The most leaf values kept.
const MAX_FIELDS: usize = 64;

/// A reply as the controller reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// `<result>`'s text, lower case; `None` when the reply has none.
    pub result: Option<String>,
    /// Every leaf element (name, text) in order, `result` included, at most 64.
    pub fields: Vec<(String, String)>,
}

impl Reply {
    /// Read `body` (see the module docs).
    pub fn parse(body: &[u8]) -> Reply {
        let body = body.get(..MAX_REPLY.min(body.len())).unwrap_or_default();
        let text = String::from_utf8_lossy(body);
        let fields = leaves(&text);
        let result = fields.iter().find(|(k, _)| k == "result").map(|(_, v)| v.trim().to_ascii_lowercase());
        Reply { result, fields }
    }

    /// The reply says `ok`.
    pub fn ok(&self) -> bool {
        self.result.as_deref() == Some("ok")
    }

    /// The camera refused us as a client: the access handshake may be needed (*hypothesis*: some
    /// firmware answers `err_unsuitable_app` or `err_reject` until `accctrl` is sent).
    pub fn wants_handshake(&self) -> bool {
        matches!(self.result.as_deref(), Some("err_unsuitable_app" | "err_reject" | "err_non_support_app"))
    }

    /// The first leaf named `name`.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Every leaf element's (name, text): an opening tag followed by text and its own closing tag.
fn leaves(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while out.len() < MAX_FIELDS {
        let Some(open) = rest.find('<') else { break };
        rest = rest.get(open + 1..).unwrap_or_default();
        let Some(close) = rest.find('>') else { break };
        let tag = rest.get(..close).unwrap_or_default();
        rest = rest.get(close + 1..).unwrap_or_default();
        let name = tag.split_whitespace().next().unwrap_or_default();
        if name.is_empty() || name.starts_with(['/', '?', '!']) || tag.ends_with('/') {
            continue;
        }
        let Some(next) = rest.find('<') else { break };
        let end_tag = format!("</{name}>");
        if rest.get(next..).is_some_and(|r| r.starts_with(&end_tag)) {
            let value = rest.get(..next).unwrap_or_default().trim();
            out.push((name.to_string(), unescape(value)));
        }
    }
    out
}

/// The five predefined XML entities.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ok_and_state() {
        let r = Reply::parse(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<camrply><result>ok</result><state><batt>3/3</batt><cammode>rec</cammode><sd_memory>set</sd_memory></state></camrply>",
        );
        assert!(r.ok());
        assert_eq!(r.field("cammode"), Some("rec"));
        assert_eq!(r.field("batt"), Some("3/3"));
        assert!(r.field("state").is_none(), "not a leaf");
    }

    #[test]
    fn reads_errors() {
        let r = Reply::parse(b"<camrply><result>err_unsuitable_app</result></camrply>");
        assert!(!r.ok());
        assert!(r.wants_handshake());
        assert!(!Reply::parse(b"<camrply><result>err_busy</result></camrply>").wants_handshake());
    }

    #[test]
    fn lenient_on_junk() {
        assert_eq!(Reply::parse(b""), Reply::default());
        assert_eq!(Reply::parse(b"<<<>>>").result, None);
        assert_eq!(Reply::parse(b"<a><b>1</a>").fields, vec![]);
        assert_eq!(Reply::parse(b"<x/><result> OK </result>").result.as_deref(), Some("ok"));
        assert_eq!(Reply::parse(b"<n>a &amp; b</n>").field("n"), Some("a & b"));
        assert_eq!(Reply::parse(&[0xFF, 0xFE, b'<']).result, None);
        let many: String = (0..200).map(|i| format!("<f{i}>v</f{i}>")).collect();
        assert_eq!(Reply::parse(many.as_bytes()).fields.len(), MAX_FIELDS);
    }
}
