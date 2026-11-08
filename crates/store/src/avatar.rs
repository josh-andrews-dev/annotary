//! Member avatars: what an avatar may be, and the [`Store`] methods that set and clear one.
//!
//! An avatar is an image (PNG, JPEG, GIF, WebP or SVG) plus an optional static fallback (PNG,
//! JPEG or WebP) a page shows under `prefers-reduced-motion: reduce`. Both are content-addressed
//! blobs; `member_avatars` (schema v6) holds one row per Member that has one. The type is always
//! the server's reading of the bytes ([`check_image`]), never a client's claim.
//!
//! **SVG is checked, never rewritten** ([`check_svg`]): the bytes are parsed as XML events (no
//! DTD, no entity expansion) and refused with a reason at the first thing off the allowlist —
//! elements [`SVG_ELEMENTS`], attributes [`SVG_ATTRIBUTES`], `href` only to `#fragment`, CSS in
//! `<style>` and `style=""` with at-rules [`CSS_AT_RULES`] and functions [`CSS_FUNCTIONS`] only and
//! `url()` only to `#fragment`. What is stored is exactly what was sent. Pages show avatars only
//! through `<img>` (no script runs there) and `/avatars/{hash}` serves them with a CSP that blocks
//! scripts and outside loads, so the check is the second wall, not the only one.

use std::borrow::Cow;

use quick_xml::events::Event;
use rusqlite::{OptionalExtension, params};

use crate::error::{Result, StoreError};
use crate::model::{Avatar, MemberView};
use crate::store::{Store, append_event, load_member};
use crate::time;

/// Largest avatar image (raster), in bytes: 256 KiB.
pub const IMAGE_MAX: u64 = 256 * 1024;
/// Largest SVG avatar, in bytes: 64 KiB.
pub const SVG_MAX: u64 = 64 * 1024;
/// Largest static fallback, in bytes: 256 KiB.
pub const STATIC_MAX: u64 = 256 * 1024;
/// Largest width or height a raster avatar may declare.
pub const MAX_DIMENSION: usize = 4096;
/// Deepest element nesting an SVG avatar may have.
pub const SVG_MAX_DEPTH: usize = 32;
/// Most elements an SVG avatar may have.
pub const SVG_MAX_ELEMENTS: usize = 4000;

pub const SVG_NS: &str = "http://www.w3.org/2000/svg";
pub const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// The only elements an SVG avatar may use.
pub const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "path",
    "circle",
    "ellipse",
    "rect",
    "polygon",
    "polyline",
    "line",
    "defs",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "style",
    "title",
];

/// The only attributes an SVG avatar may use (on any allowed element): presentation and
/// geometry, gradient and clip parameters, ARIA labels, and the two namespace declarations.
pub const SVG_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "style",
    "transform",
    "transform-origin",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-opacity",
    "opacity",
    "clip-path",
    "clip-rule",
    "display",
    "visibility",
    "color",
    "vector-effect",
    "shape-rendering",
    "paint-order",
    "pathLength",
    "x",
    "y",
    "width",
    "height",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x1",
    "y1",
    "x2",
    "y2",
    "d",
    "points",
    "viewBox",
    "preserveAspectRatio",
    "version",
    "role",
    "aria-label",
    "aria-hidden",
    "focusable",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "fx",
    "fy",
    "fr",
    "offset",
    "stop-color",
    "stop-opacity",
    "clipPathUnits",
    "type",
    "media",
    "xml:space",
    "href",
    "xlink:href",
    "xmlns",
    "xmlns:xlink",
];

/// The only CSS at-rules an SVG avatar's styles may use.
pub const CSS_AT_RULES: &[&str] = &["keyframes", "-webkit-keyframes", "media"];

/// The only CSS functions an SVG avatar's styles may call (`url()` only to `#fragment`).
pub const CSS_FUNCTIONS: &[&str] = &[
    "url",
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "oklab",
    "oklch",
    "color-mix",
    "var",
    "calc",
    "min",
    "max",
    "clamp",
    "translate",
    "translatex",
    "translatey",
    "translatez",
    "translate3d",
    "rotate",
    "rotatex",
    "rotatey",
    "rotatez",
    "rotate3d",
    "scale",
    "scalex",
    "scaley",
    "scalez",
    "scale3d",
    "skew",
    "skewx",
    "skewy",
    "matrix",
    "matrix3d",
    "perspective",
    "cubic-bezier",
    "steps",
    "linear",
    "color",
    "light-dark",
    "drop-shadow",
    "blur",
    "brightness",
    "contrast",
    "grayscale",
    "hue-rotate",
    "invert",
    "opacity",
    "saturate",
    "sepia",
    // Selectors and media conditions, which load nothing.
    "not",
    "is",
    "where",
    "has",
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
];

/// What the bytes of an avatar are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
}

impl Kind {
    pub fn content_type(self) -> &'static str {
        match self {
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Gif => "image/gif",
            Kind::Webp => "image/webp",
            Kind::Svg => "image/svg+xml",
        }
    }
}

/// Why an avatar was refused: a reason for people (400 `invalid_avatar`), or a size over its cap
/// (413 `too_large`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Invalid(String),
    TooLarge { size: u64, max: u64 },
}

impl Refusal {
    fn invalid(reason: impl Into<String>) -> Refusal {
        Refusal::Invalid(reason.into())
    }

    /// The store error for the image (`what` = `the avatar` / `the static avatar`).
    pub fn into_store_error(self, what: &str) -> StoreError {
        match self {
            Refusal::Invalid(reason) => StoreError::InvalidAvatar(format!("{what}: {reason}")),
            Refusal::TooLarge { size, max } => StoreError::TooLarge { what: what.to_string(), size, max },
        }
    }
}

/// The raster kind the bytes start with, if any; `Svg` when they start like XML.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Kind::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Kind::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(Kind::Webp);
    }
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let first = text.iter().find(|b| !b.is_ascii_whitespace());
    (first == Some(&b'<')).then_some(Kind::Svg)
}

/// Check an avatar image: its kind (by its bytes) if it is a PNG, JPEG, GIF or WebP up to
/// [`IMAGE_MAX`] with dimensions its header gives, or an SVG up to [`SVG_MAX`] that passes
/// [`check_svg`].
pub fn check_image(bytes: &[u8]) -> std::result::Result<Kind, Refusal> {
    let size = bytes.len() as u64;
    match sniff(bytes) {
        Some(Kind::Svg) => {
            if size > SVG_MAX {
                return Err(Refusal::TooLarge { size, max: SVG_MAX });
            }
            check_svg(bytes)?;
            Ok(Kind::Svg)
        }
        Some(kind) => {
            if size > IMAGE_MAX {
                return Err(Refusal::TooLarge { size, max: IMAGE_MAX });
            }
            check_raster(bytes, kind)?;
            Ok(kind)
        }
        None => Err(Refusal::invalid("not a PNG, JPEG, GIF, WebP or SVG image")),
    }
}

/// Check a static fallback: a PNG, JPEG or WebP up to [`STATIC_MAX`] (no SVG and no GIF: what
/// shows under reduced motion must not move).
pub fn check_static(bytes: &[u8]) -> std::result::Result<Kind, Refusal> {
    let size = bytes.len() as u64;
    match sniff(bytes) {
        Some(kind @ (Kind::Png | Kind::Jpeg | Kind::Webp)) => {
            if size > STATIC_MAX {
                return Err(Refusal::TooLarge { size, max: STATIC_MAX });
            }
            check_raster(bytes, kind)?;
            Ok(kind)
        }
        Some(Kind::Svg | Kind::Gif) => {
            Err(Refusal::invalid("the static fallback must be a PNG, JPEG or WebP (an SVG or GIF may move)"))
        }
        None => Err(Refusal::invalid("not a PNG, JPEG or WebP image")),
    }
}

fn check_raster(bytes: &[u8], kind: Kind) -> std::result::Result<(), Refusal> {
    let name = match kind {
        Kind::Png => "PNG",
        Kind::Jpeg => "JPEG",
        Kind::Gif => "GIF",
        Kind::Webp => "WebP",
        Kind::Svg => "SVG",
    };
    match imagesize::blob_size(bytes) {
        Ok(d) if d.width == 0 || d.height == 0 => Err(Refusal::invalid(format!("the {name} has no width or height"))),
        Ok(d) if d.width > MAX_DIMENSION || d.height > MAX_DIMENSION => Err(Refusal::invalid(format!(
            "the {name} is {}×{}; the most is {MAX_DIMENSION}×{MAX_DIMENSION}",
            d.width, d.height
        ))),
        Ok(_) => Ok(()),
        Err(_) => Err(Refusal::invalid(format!("the {name} cannot be read (damaged or cut short)"))),
    }
}

// ---------------------------------------------------------------- the SVG check

/// Refuse an SVG avatar at the first thing off the allowlist (see the module docs). `Ok` means
/// the bytes are well-formed UTF-8 XML whose root is `<svg xmlns="http://www.w3.org/2000/svg">`
/// and which uses only allowed elements, attributes, references and CSS.
pub fn check_svg(bytes: &[u8]) -> std::result::Result<(), Refusal> {
    if bytes.len() as u64 > SVG_MAX {
        return Err(Refusal::TooLarge { size: bytes.len() as u64, max: SVG_MAX });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| Refusal::invalid("the SVG is not UTF-8"))?;
    if text.chars().any(|c| (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')) {
        return Err(Refusal::invalid("the SVG has control characters"));
    }
    let mut reader = quick_xml::Reader::from_str(text);
    {
        let cfg = reader.config_mut();
        cfg.check_comments = true;
        cfg.check_end_names = true;
        cfg.allow_unmatched_ends = false;
    }
    let mut stack: Vec<String> = Vec::new();
    let mut elements = 0usize;
    let mut root_done = false;
    let mut first = true;
    let mut style_text = String::new();
    loop {
        let ev = reader.read_event().map_err(|e| Refusal::invalid(format!("the SVG is not well-formed XML ({e})")))?;
        let was_first = std::mem::replace(&mut first, false);
        match ev {
            Event::Decl(d) => {
                if !was_first {
                    return Err(Refusal::invalid("an XML declaration is allowed only at the start"));
                }
                if d.version().map_err(|_| Refusal::invalid("the XML declaration cannot be read"))?.as_ref() != "1.0" {
                    return Err(Refusal::invalid("the SVG must be XML 1.0"));
                }
                if let Some(enc) = d.encoding() {
                    let enc = enc.map_err(|_| Refusal::invalid("the XML declaration cannot be read"))?;
                    if !enc.eq_ignore_ascii_case("utf-8") {
                        return Err(Refusal::invalid(format!("the SVG must be UTF-8, not {enc}")));
                    }
                }
            }
            Event::DocType(_) => {
                return Err(Refusal::invalid("a DOCTYPE is not allowed (no DTDs or entity declarations)"));
            }
            Event::PI(_) => return Err(Refusal::invalid("a processing instruction is not allowed")),
            Event::Comment(_) => {}
            Event::Start(e) | Event::Empty(e) if root_done => {
                let _ = e;
                return Err(Refusal::invalid("the SVG has content after its root element"));
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let name = e.name();
                let name = name.as_ref();
                elements += 1;
                if elements > SVG_MAX_ELEMENTS {
                    return Err(Refusal::invalid(format!("the SVG has more than {SVG_MAX_ELEMENTS} elements")));
                }
                if stack.is_empty() && name != "svg" {
                    return Err(Refusal::invalid(format!("the root element must be <svg>, not <{name}>")));
                }
                if !SVG_ELEMENTS.contains(&name) {
                    return Err(Refusal::invalid(format!("the <{name}> element is not allowed")));
                }
                if !stack.is_empty() && name == "svg" {
                    return Err(Refusal::invalid("a nested <svg> is not allowed"));
                }
                if matches!(stack.last().map(String::as_str), Some("style" | "title")) {
                    return Err(Refusal::invalid(format!(
                        "<{}> may hold only text, not <{name}>",
                        stack.last().map(String::as_str).unwrap_or("")
                    )));
                }
                check_attributes(e, stack.is_empty())?;
                if empty {
                    if stack.is_empty() {
                        root_done = true;
                    }
                } else {
                    stack.push(name.to_string());
                    if stack.len() > SVG_MAX_DEPTH {
                        return Err(Refusal::invalid(format!("the SVG nests deeper than {SVG_MAX_DEPTH} elements")));
                    }
                }
            }
            Event::End(_) => {
                if stack.last().map(String::as_str) == Some("style") {
                    check_css(&style_text, CssPlace::Sheet)?;
                    style_text.clear();
                }
                stack.pop();
                if stack.is_empty() {
                    root_done = true;
                }
            }
            Event::Text(t) => {
                let raw: &str = &t;
                match stack.last().map(String::as_str) {
                    Some("style") => style_text.push_str(raw),
                    Some("title") => {}
                    _ if raw.trim().is_empty() => {}
                    _ => return Err(Refusal::invalid("text is allowed only inside <style> and <title>")),
                }
            }
            Event::CData(c) => match stack.last().map(String::as_str) {
                Some("style") => style_text.push_str(&c),
                _ => return Err(Refusal::invalid("CDATA is allowed only inside <style>")),
            },
            Event::GeneralRef(r) => {
                let resolved: Cow<str> = if r.is_char_ref() {
                    match r.resolve_char_ref() {
                        Ok(Some(c)) => Cow::Owned(c.to_string()),
                        _ => return Err(Refusal::invalid(format!("the character reference &{}; is not valid", &*r))),
                    }
                } else {
                    match &*r {
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "amp" => "&".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        other => return Err(Refusal::invalid(format!("the entity &{other}; is not allowed"))),
                    }
                };
                match stack.last().map(String::as_str) {
                    Some("style") => style_text.push_str(&resolved),
                    Some("title") => {}
                    _ if resolved.trim().is_empty() => {}
                    _ => return Err(Refusal::invalid("text is allowed only inside <style> and <title>")),
                }
            }
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(Refusal::invalid(format!(
            "the SVG ends inside <{}>",
            stack.last().map(String::as_str).unwrap_or("")
        )));
    }
    if !root_done {
        return Err(Refusal::invalid("there is no <svg> element"));
    }
    Ok(())
}

fn check_attributes(e: &quick_xml::events::BytesStart<'_>, root: bool) -> std::result::Result<(), Refusal> {
    let mut has_ns = false;
    let mut attrs = e.attributes();
    attrs.with_checks(true);
    for a in attrs {
        let a = a.map_err(|err| Refusal::invalid(format!("an attribute cannot be read ({err})")))?;
        let key = a.key.as_ref();
        if !SVG_ATTRIBUTES.contains(&key) {
            return Err(Refusal::invalid(format!("the {key} attribute is not allowed")));
        }
        let value = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| Refusal::invalid(format!("the {key} value cannot be read ({err})")))?;
        let value = value.as_ref();
        match key {
            "xmlns" if value != SVG_NS => {
                return Err(Refusal::invalid(format!("xmlns must be {SVG_NS}")));
            }
            "xmlns" => has_ns = true,
            "xmlns:xlink" if value != XLINK_NS => {
                return Err(Refusal::invalid(format!("xmlns:xlink must be {XLINK_NS}")));
            }
            "href" | "xlink:href" if !is_fragment(value.trim()) => {
                return Err(Refusal::invalid(format!("{key} may point only inside the SVG (#id), not {value:?}")));
            }
            "style" => check_css(value, CssPlace::Declarations)?,
            "type" if !value.trim().eq_ignore_ascii_case("text/css") => {
                return Err(Refusal::invalid("<style type> must be text/css"));
            }
            "media" => check_css(value, CssPlace::Declarations)?,
            // Words for a screen reader, never parsed as a reference or CSS.
            "aria-label" => {}
            _ => check_value(key, value)?,
        }
    }
    if root && !has_ns {
        return Err(Refusal::invalid(format!("the root <svg> needs xmlns=\"{SVG_NS}\"")));
    }
    Ok(())
}

/// `#id` (an XML name after the hash).
fn is_fragment(v: &str) -> bool {
    let Some(id) = v.strip_prefix('#') else { return false };
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

/// A presentation value: no scheme that could run or load, and `url()` only to `#fragment`.
fn check_value(key: &str, value: &str) -> std::result::Result<(), Refusal> {
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    for scheme in ["javascript:", "vbscript:", "data:"] {
        if compact.contains(scheme) {
            return Err(Refusal::invalid(format!("the {key} value names {scheme}")));
        }
    }
    if value.contains('\\') {
        return Err(Refusal::invalid(format!("the {key} value has a backslash escape")));
    }
    check_functions(value)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CssPlace {
    /// A `<style>` sheet: rules, `@keyframes`, `@media`.
    Sheet,
    /// A `style=""` attribute or a `media=""` query: no at-rules at all.
    Declarations,
}

/// The CSS of a `<style>` or a `style=""`: comments removed first (so a comment cannot split a
/// word the checks look for), then no backslash escape, no at-rule but [`CSS_AT_RULES`] (none in
/// an attribute), and only [`CSS_FUNCTIONS`] with `url()` to `#fragment`.
fn check_css(css: &str, place: CssPlace) -> std::result::Result<(), Refusal> {
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        text.push_str(&rest[..at]);
        match rest[at + 2..].find("*/") {
            Some(end) => rest = &rest[at + 2 + end + 2..],
            None => return Err(Refusal::invalid("a CSS comment is not closed")),
        }
    }
    text.push_str(rest);
    if text.contains('\\') {
        return Err(Refusal::invalid("CSS escapes (backslashes) are not allowed"));
    }
    let lower = text.to_ascii_lowercase();
    for scheme in ["javascript:", "vbscript:", "data:"] {
        if lower.chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(scheme) {
            return Err(Refusal::invalid(format!("the CSS names {scheme}")));
        }
    }
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let name: String =
                lower[i + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if place == CssPlace::Declarations || !CSS_AT_RULES.contains(&name.as_str()) {
                return Err(Refusal::invalid(format!("the CSS rule @{name} is not allowed")));
            }
            i += 1 + name.len();
        } else {
            i += 1;
        }
    }
    check_functions(&text)
}

/// Every `name(` in `text` must be in [`CSS_FUNCTIONS`], and every `url(…)` a `#fragment`.
fn check_functions(text: &str) -> std::result::Result<(), Refusal> {
    let lower = text.to_ascii_lowercase();
    let mut search = 0;
    while let Some(off) = lower[search..].find('(') {
        let open = search + off;
        let name: String = lower[..open]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !name.is_empty() && !CSS_FUNCTIONS.contains(&name.as_str()) {
            return Err(Refusal::invalid(format!("the CSS function {name}() is not allowed")));
        }
        if name == "url" {
            let close =
                lower[open..].find(')').map(|c| open + c).ok_or_else(|| Refusal::invalid("a url( is not closed"))?;
            let inner = text[open + 1..close].trim();
            let inner = inner
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(inner)
                .trim();
            if !is_fragment(inner) {
                return Err(Refusal::invalid(format!("url() may point only inside the SVG (#id), not {inner:?}")));
            }
        }
        search = open + 1;
    }
    Ok(())
}

// ---------------------------------------------------------------- the store

/// The avatar of a Member row selected with `store::MEMBER_SELECT` (its `av_*` columns, read by
/// name); `None` when it has none, or when the query did not select them.
pub(crate) fn row_avatar(r: &rusqlite::Row<'_>) -> Option<Avatar> {
    let hash: String = r.get::<_, Option<String>>("av_hash").ok().flatten()?;
    Some(Avatar {
        hash,
        content_type: r.get("av_type").ok()?,
        size: r.get::<_, i64>("av_size").ok()? as u64,
        static_hash: r.get("av_static_hash").ok().flatten(),
        static_type: r.get("av_static_type").ok().flatten(),
        static_size: r.get::<_, Option<i64>>("av_static_size").ok().flatten().map(|s| s as u64),
        set_at: r.get("av_set_at").ok()?,
    })
}

impl Store {
    /// Set a Member's avatar (and its optional static fallback) — `by` is the Member themselves
    /// (any Member, through `/api/v1/me/avatar`). Both images are checked ([`check_image`],
    /// [`check_static`]) and stored as blobs before the transaction; the same images as now change
    /// nothing (`false`, no event), else `member.avatar_set {member, handle, hash, type,
    /// static_hash, static_type}`.
    pub fn set_avatar(
        &self,
        by: &str,
        member_id: &str,
        image: &[u8],
        static_image: Option<&[u8]>,
    ) -> Result<(MemberView, bool)> {
        let kind = check_image(image).map_err(|r| r.into_store_error("the avatar"))?;
        let static_kind = match static_image {
            Some(b) => Some(check_static(b).map_err(|r| r.into_store_error("the static avatar"))?),
            None => None,
        };
        let content_type = kind.content_type();
        let static_type = static_kind.map(Kind::content_type);
        self.tx(|tx| {
            // Written inside the transaction: the GC holds the same connection, so it cannot
            // sweep a blob a cleared avatar left behind between this write and the commit.
            let (hash, _) = self.blobs.put(image)?;
            let static_hash = match static_image {
                Some(b) => Some(self.blobs.put(b)?.0),
                None => None,
            };
            let m = load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
            if let Some(a) = &m.avatar
                && a.hash == hash
                && a.static_hash == static_hash
            {
                return Ok((m, false));
            }
            tx.execute(
                "INSERT INTO member_avatars(member_id, hash, type, size, static_hash, static_type, static_size, set_at, set_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(member_id) DO UPDATE SET hash = excluded.hash, type = excluded.type, size = excluded.size,
                   static_hash = excluded.static_hash, static_type = excluded.static_type,
                   static_size = excluded.static_size, set_at = excluded.set_at, set_by = excluded.set_by",
                params![
                    member_id,
                    hash,
                    content_type,
                    image.len() as i64,
                    static_hash,
                    static_type,
                    static_image.map(|b| b.len() as i64),
                    time::now(),
                    by
                ],
            )?;
            append_event(
                tx,
                Some(by),
                "member.avatar_set",
                None,
                serde_json::json!({
                    "member": member_id,
                    "handle": m.handle,
                    "hash": hash,
                    "type": content_type,
                    "static_hash": static_hash,
                    "static_type": static_type,
                }),
            )?;
            Ok((load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?, true))
        })
    }

    /// Clear a Member's avatar (the Member themselves, or an owner for anyone — the server checks
    /// which). No avatar ⇒ `false`, no event; else `member.avatar_cleared {member, handle}`. The
    /// blobs are left to the GC.
    pub fn clear_avatar(&self, by: &str, member_id: &str) -> Result<(MemberView, bool)> {
        self.tx(|tx| {
            let m = load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
            if m.avatar.is_none() {
                return Ok((m, false));
            }
            tx.execute("DELETE FROM member_avatars WHERE member_id = ?1", [member_id])?;
            append_event(
                tx,
                Some(by),
                "member.avatar_cleared",
                None,
                serde_json::json!({ "member": member_id, "handle": m.handle }),
            )?;
            Ok((load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?, true))
        })
    }

    /// The content type of `hash` when it is some Member's current avatar or static fallback
    /// (what `/avatars/{hash}` may serve); `None` for any other blob.
    pub fn avatar_type(&self, hash: &str) -> Result<Option<String>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT type FROM member_avatars WHERE hash = ?1
                 UNION ALL SELECT static_type FROM member_avatars WHERE static_hash = ?1 LIMIT 1",
                [hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }
}

/// Every blob a current avatar uses (the GC and `export` keep them: `docs::referenced_hashes`).
/// `export` reads stores older than v6 without migrating them, so a missing table is no avatars.
pub(crate) fn avatar_blob_refs(tx: &rusqlite::Transaction) -> Result<Vec<String>> {
    let has_table = tx
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'member_avatars'", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !has_table {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare("SELECT hash, static_hash FROM member_avatars")?;
    let mut out = Vec::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))? {
        let (h, s) = row?;
        out.push(h);
        out.extend(s);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the Hermes plugin writes: keyframes, a reduced-motion rule, nested animated groups.
    pub const ANIMATED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- a test face -->
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 64 64" width="64" height="64" role="img" aria-label="Scribe">
  <title>Scribe</title>
  <style>
    @keyframes blink { 0%, 92%, 100% { transform: scaleY(1) } 96% { transform: scaleY(0.1) } }
    @keyframes bob { 0%, 100% { transform: translateY(0) } 50% { transform: translateY(-2px) } }
    .head { animation: bob 3s ease-in-out infinite; transform-origin: 32px 32px; }
    .eye { animation: blink 4s steps(1) infinite; transform-box: fill-box; transform-origin: center; fill: url(#g) }
    @media (prefers-reduced-motion: reduce) { .head, .eye { animation: none } }
  </style>
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4b6"/><stop offset="1" stop-color="rgb(20, 80, 40)"/></linearGradient>
    <radialGradient id="r" xlink:href="#g" cx="0.5" cy="0.5" r="0.5"/>
    <clipPath id="c"><circle cx="32" cy="32" r="30"/></clipPath>
  </defs>
  <g clip-path="url(#c)">
    <rect width="64" height="64" fill="url(#r)"/>
    <g class="head" style="animation-delay: 0.2s">
      <ellipse cx="32" cy="34" rx="20" ry="18" fill="#fde"/>
      <g><circle class="eye" cx="25" cy="30" r="3"/><circle class="eye" cx="39" cy="30" r="3"/></g>
      <path d="M24 42 Q32 48 40 42" stroke="#333" stroke-width="2" fill="none" stroke-linecap="round"/>
      <polygon points="30,36 34,36 32,39" fill="#c88"/><polyline points="1,1 2,2" stroke="#000"/><line x1="0" y1="0" x2="1" y2="1"/>
    </g>
  </g>
</svg>
"##;

    fn refused(svg: &str) -> String {
        match check_svg(svg.as_bytes()) {
            Err(Refusal::Invalid(r)) => r,
            other => panic!("expected a refusal for {svg:?}, got {other:?}"),
        }
    }

    fn wrap(inner: &str) -> String {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\">{inner}</svg>")
    }

    // spec: S41-01
    #[test]
    fn the_plugin_shape_passes_and_is_an_svg() {
        assert_eq!(check_svg(ANIMATED.as_bytes()), Ok(()));
        // Selectors, filters and a label with parentheses load nothing.
        let more = wrap(
            "<style>.a:not(.b):nth-child(2) { filter: drop-shadow(0 0 1px #000) }</style><g aria-label=\"Scribe (agent): call(me)\"/>",
        );
        assert_eq!(check_svg(more.as_bytes()), Ok(()));
        assert_eq!(check_image(ANIMATED.as_bytes()), Ok(Kind::Svg));
        // A BOM, CDATA in the style, entities and char refs in the style, a self-closed root.
        let cdata = "\u{FEFF}<svg xmlns=\"http://www.w3.org/2000/svg\"><style><![CDATA[ .a > .b { fill: #000 } ]]>.c{opacity:&#48;.5}</style><g class=\"a\"><rect class=\"b\" width=\"1\" height=\"1\"/></g></svg>";
        assert_eq!(check_svg(cdata.as_bytes()), Ok(()));
        assert_eq!(check_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Ok(()));
    }

    // spec: S41-01
    #[test]
    fn scripts_events_and_foreign_content_are_refused() {
        assert!(refused(&wrap("<script>alert(1)</script>")).contains("<script>"));
        assert!(refused(&wrap("<g onload=\"alert(1)\"/>")).contains("onload"));
        assert!(refused("<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"/>").contains("onload"));
        assert!(refused(&wrap("<g onClick=\"x\"/>")).contains("onClick"));
        assert!(
            refused(&wrap("<foreignObject><div xmlns=\"http://www.w3.org/1999/xhtml\">x</div></foreignObject>"))
                .contains("foreignObject")
        );
        assert!(refused(&wrap("<image href=\"https://example.com/x.png\"/>")).contains("<image>"));
        assert!(refused(&wrap("<use href=\"#a\"/>")).contains("<use>"));
        assert!(refused(&wrap("<a href=\"javascript:alert(1)\"><rect/></a>")).contains("<a>"));
        assert!(refused(&wrap("<animate attributeName=\"href\" to=\"javascript:alert(1)\"/>")).contains("<animate>"));
        assert!(refused(&wrap("<set attributeName=\"onload\"/>")).contains("<set>"));
        assert!(refused(&wrap("<svg/>")).contains("nested"));
        assert!(refused(&wrap("<html:script xmlns:html=\"http://www.w3.org/1999/xhtml\"/>")).contains("not allowed"));
        assert!(refused(&wrap("<g xmlns=\"http://www.w3.org/1999/xhtml\"/>")).contains("xmlns"));
        assert!(refused(&wrap("<style><g/></style>")).contains("only text"));
        assert!(refused(&wrap("hello")).contains("text is allowed only"));
        assert!(refused(&wrap("<![CDATA[x]]>")).contains("CDATA"));
    }

    // spec: S41-01
    #[test]
    fn references_out_of_the_svg_are_refused() {
        let grad = |attr: &str| {
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><linearGradient {attr}/></svg>"
            )
        };
        assert_eq!(check_svg(grad("href=\"#g\"").as_bytes()), Ok(()));
        assert!(refused(&grad("href=\"https://example.com/a.svg#g\"")).contains("href"));
        assert!(refused(&grad("xlink:href=\"javascript:alert(1)\"")).contains("xlink:href"));
        assert!(refused(&grad("xlink:href=\"data:image/svg+xml,&lt;svg/&gt;\"")).contains("xlink:href"));
        assert!(refused(&grad("href=\"other.svg#g\"")).contains("href"));
        assert!(refused(&grad("href=\" java&#x09;script:alert(1)\"")).contains("href"));
        assert!(refused(&grad("xml:base=\"https://example.com/\"")).contains("xml:base"));
        assert!(refused(&grad("xmlns:evil=\"https://example.com/ns\"")).contains("xmlns:evil"));
        assert!(refused(&wrap("<rect fill=\"url(https://example.com/x.svg#a)\"/>")).contains("url()"));
        assert!(refused(&wrap("<rect fill=\"javascript:alert(1)\"/>")).contains("javascript:"));
        assert!(refused(&wrap("<rect style=\"fill:url('http://example.com/a.svg#x')\"/>")).contains("url()"));
        assert!(refused("<svg viewBox=\"0 0 1 1\"/>").contains("needs xmlns"));
        assert!(refused("<svg xmlns=\"http://www.w3.org/1999/xhtml\"/>").contains("xmlns must be"));
        assert!(refused("<g xmlns=\"http://www.w3.org/2000/svg\"/>").contains("root element must be <svg>"));
    }

    // spec: S41-01
    #[test]
    fn css_that_loads_or_escapes_is_refused() {
        let style = |css: &str| wrap(&format!("<style>{css}</style>"));
        assert!(refused(&style("@import url(https://example.com/x.css);")).contains("@import"));
        assert!(refused(&style("@import 'x.css';")).contains("@import"));
        assert!(refused(&style("@font-face { font-family: x; src: url(#f) }")).contains("@font-face"));
        assert!(refused(&style("@namespace svg url(#x);")).contains("@namespace"));
        assert!(refused(&style(".a { fill: url(http://example.com/a.svg#x) }")).contains("url()"));
        assert!(refused(&style(".a { fill: url( \"//example.com/a\" ) }")).contains("url()"));
        assert!(refused(&style(".a { background: image-set('x.png' 1x) }")).contains("image-set()"));
        assert!(refused(&style(".a { width: expression(alert(1)) }")).contains("expression()"));
        assert!(refused(&style(".a { fill: u\\72l(http://example.com) }")).contains("escape"));
        assert!(refused(&style("@\\69mport 'x';")).contains("escape"));
        assert!(refused(&style("@im/**/port 'x';")).contains("@import"));
        assert!(refused(&style(".a { fill: ur/**/l(http://example.com/x) }")).contains("url()"));
        assert!(refused(&style("/* never closed")).contains("not closed"));
        // An entity or char ref cannot hide an at-rule from the check: they are resolved first.
        assert!(refused(&style("&#64;import 'x.css';")).contains("@import"));
        assert!(refused(&wrap("<rect style=\"@import 'x'\"/>")).contains("@import"));
        assert!(refused(&wrap("<style media=\"screen and (min-width: url(http://x))\"/>")).contains("url()"));
        assert!(refused(&wrap("<style type=\"text/javascript\">x</style>")).contains("text/css"));
        // The reduced-motion rule and keyframes stay allowed.
        assert_eq!(
            check_svg(style("@media (prefers-reduced-motion: reduce) { * { animation: none !important } }").as_bytes()),
            Ok(())
        );
    }

    // spec: S41-01
    #[test]
    fn dtds_entities_and_oddities_are_refused() {
        let xxe = "<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><svg xmlns=\"http://www.w3.org/2000/svg\"><title>&x;</title></svg>";
        assert!(refused(xxe).contains("DOCTYPE"));
        let laughs = "<!DOCTYPE svg [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\">]><svg xmlns=\"http://www.w3.org/2000/svg\"><title>&b;</title></svg>";
        assert!(refused(laughs).contains("DOCTYPE"));
        assert!(refused(&wrap("<title>&nbsp;</title>")).contains("&nbsp;"));
        assert!(refused(&wrap("<style>&x;</style>")).contains("&x;"));
        assert!(refused(&wrap("<rect fill=\"&x;\"/>")).contains("fill"));
        assert!(refused(&format!("<?xml-stylesheet href=\"x.css\"?>{}", wrap(""))).contains("processing instruction"));
        assert!(
            refused(&format!("{}<?php echo 1 ?>", wrap(""))).contains("after its root")
                || refused(&format!("{}<?php echo 1 ?>", wrap(""))).contains("processing instruction")
        );
        assert!(refused(&format!("{}{}", wrap(""), wrap(""))).contains("after its root"));
        assert!(
            refused("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>")
                .contains("UTF-8")
        );
        assert!(
            refused(&format!(" {}", "<!-- x --><?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>"))
                .contains("declaration")
        );
        assert!(refused(&wrap("<g>")).contains("not well-formed") || refused(&wrap("<g>")).contains("ends inside"));
        assert!(refused("<svg xmlns=\"http://www.w3.org/2000/svg\"><g></svg>").contains("not well-formed"));
        assert!(refused(&wrap("<rect x=\"1\" x=\"2\"/>")).contains("attribute"));
        assert!(refused(&wrap("<!-- a -- b -->")).contains("well-formed"));
        assert!(refused(&wrap("<rect fill=\"a\u{0}\"/>")).contains("control"));
        assert!(
            matches!(check_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\">\xff</svg>"), Err(Refusal::Invalid(r)) if r.contains("UTF-8"))
        );
        let deep = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(SVG_MAX_DEPTH + 1),
            "</g>".repeat(SVG_MAX_DEPTH + 1)
        );
        assert!(refused(&deep).contains("deeper"));
        let many = wrap(&"<rect/>".repeat(SVG_MAX_ELEMENTS + 1));
        assert!(
            matches!(check_svg(many.as_bytes()), Err(Refusal::Invalid(r)) if r.contains("elements"))
                || many.len() as u64 > SVG_MAX
        );
        let big = wrap(&format!("<title>{}</title>", "x".repeat(SVG_MAX as usize)));
        assert_eq!(check_svg(big.as_bytes()), Err(Refusal::TooLarge { size: big.len() as u64, max: SVG_MAX }));
        assert_eq!(check_image(big.as_bytes()), Err(Refusal::TooLarge { size: big.len() as u64, max: SVG_MAX }));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    // spec: S41-02
    #[test]
    fn rasters_are_known_by_their_bytes_and_capped() {
        assert_eq!(check_image(&png(64, 64)), Ok(Kind::Png));
        assert_eq!(check_static(&png(64, 64)), Ok(Kind::Png));
        let gif = b"GIF89a\x40\x00\x40\x00\x80\x00\x00".to_vec();
        assert_eq!(check_image(&gif), Ok(Kind::Gif));
        assert!(matches!(check_static(&gif), Err(Refusal::Invalid(r)) if r.contains("PNG, JPEG or WebP")));
        assert!(matches!(check_static(ANIMATED.as_bytes()), Err(Refusal::Invalid(r)) if r.contains("may move")));
        assert!(matches!(check_image(b"%PDF-1.4"), Err(Refusal::Invalid(r)) if r.contains("not a PNG")));
        assert!(matches!(check_image(b""), Err(Refusal::Invalid(_))));
        assert!(matches!(check_image(b"\x89PNG\r\n\x1a\n"), Err(Refusal::Invalid(r)) if r.contains("cannot be read")));
        assert!(matches!(check_image(&png(0, 4)), Err(Refusal::Invalid(r)) if r.contains("no width")));
        assert!(matches!(check_image(&png(5000, 4)), Err(Refusal::Invalid(r)) if r.contains("5000")));
        let mut big = png(64, 64);
        big.resize(IMAGE_MAX as usize + 1, 0);
        assert_eq!(check_image(&big), Err(Refusal::TooLarge { size: IMAGE_MAX + 1, max: IMAGE_MAX }));
        assert_eq!(check_static(&big), Err(Refusal::TooLarge { size: STATIC_MAX + 1, max: STATIC_MAX }));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Webp));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Kind::Jpeg));
        assert_eq!(sniff(b"  <svg"), Some(Kind::Svg));
    }
}
