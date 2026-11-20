//! Markup: a drawing a Member made over the rendered Document (in the app), kept on one Comment
//! as two content-addressed blobs: a PNG of the marked region with the ink on it, and the strokes
//! as JSON ([`check_strokes`] has the shape). `docs/dev/markup.md` has the why.
//!
//! Upload and attach are two steps, like a Version's blobs and its commit: [`Store::upload_markup`]
//! checks both files and stores them as blobs (no row, no event) and answers the Markup's id,
//! `<image hash>.<strokes hash>`; a new Thread or a Reply names that id, and the Comment's
//! transaction checks both blobs again and writes the `comment_markups` row (schema v7) with the
//! Comment's own `thread.created` / `thread.replied` event. An id nobody attaches within the GC's
//! grace period loses its blobs (409 `markup_missing` then; upload again).

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::blobs::is_blob_hash;
use crate::error::{Result, StoreError};
use crate::store::Store;

/// Largest Markup image, in bytes: 2 MiB.
pub const IMAGE_MAX: u64 = 2 * 1024 * 1024;
/// Largest strokes file, in bytes: 512 KiB.
pub const STROKES_MAX: u64 = 512 * 1024;
/// Largest width or height of a Markup image, in pixels.
pub const MAX_SIDE: u32 = 4096;
/// Most bytes the PNG decoder may allocate while checking an image (a 4,096 px square in 16-bit
/// RGBA is 128 MiB; the decoder holds rows, not the frame, but its limit counts the frame).
const DECODE_LIMIT: usize = 160 * 1024 * 1024;

/// A Markup on a Comment, as read back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Markup {
    /// The PNG's blob hash.
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub image_size: u64,
    /// The strokes JSON's blob hash.
    pub strokes: String,
    pub strokes_size: u64,
}

/// What an upload answers: the id a Thread or Reply names, and what was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupUpload {
    pub id: String,
    pub markup: Markup,
}

/// The id of a Markup made of these two blobs.
pub fn markup_id(image: &str, strokes: &str) -> String {
    format!("{image}.{strokes}")
}

/// `(image hash, strokes hash)` of a Markup id, if it is one.
pub fn parse_markup_id(id: &str) -> Option<(&str, &str)> {
    let (image, strokes) = id.trim().split_once('.')?;
    (is_blob_hash(image) && is_blob_hash(strokes)).then_some((image, strokes))
}

fn invalid(reason: impl Into<String>) -> StoreError {
    StoreError::InvalidMarkup(reason.into())
}

/// Check a Markup image: a PNG of at most [`IMAGE_MAX`] bytes and [`MAX_SIDE`] pixels a side
/// that decodes whole (every row, and the checksums up to `IEND`). Answers `(width, height)`.
pub fn check_png(bytes: &[u8]) -> Result<(u32, u32)> {
    let size = bytes.len() as u64;
    if size > IMAGE_MAX {
        return Err(StoreError::TooLarge { what: "the Markup image".into(), size, max: IMAGE_MAX });
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid("the Markup image is not a PNG"));
    }
    let damaged = |_| invalid("the Markup image cannot be read as a PNG (damaged or cut short)");
    let decoder = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: DECODE_LIMIT });
    let mut reader = decoder.read_info().map_err(damaged)?;
    let (width, height) = reader.info().size();
    if width == 0 || height == 0 {
        return Err(invalid("the Markup image has no width or height"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(invalid(format!("the Markup image is {width}×{height}; the most is {MAX_SIDE}×{MAX_SIDE}")));
    }
    while reader.next_row().map_err(damaged)?.is_some() {}
    reader.finish().map_err(damaged)?;
    Ok((width, height))
}

/// Check a strokes file: UTF-8 JSON of at most [`STROKES_MAX`] bytes, of this shape and no other
/// keys:
///
/// ```json
/// {"strokes": [{"color": "#1f6feb", "width": 3, "tool": "pen", "points": [[12.5, 40, 0], [13, 41.5, 0.016, 0.7]]}]}
/// ```
///
/// One or more strokes; `color` is `#rrggbb` or `#rrggbbaa`; `width` is the line width in image
/// pixels, above 0 and at most 1,000; `tool` is optional, 1 to 32 of `a-z` and `-`; `points` holds
/// one or more `[x, y, t]` or `[x, y, t, force]`, finite numbers, `x` / `y` in the PNG's pixels
/// from its top left, `t` seconds since the Markup's first point (0 or more), `force` 0 or more.
pub fn check_strokes(bytes: &[u8]) -> Result<()> {
    let size = bytes.len() as u64;
    if size > STROKES_MAX {
        return Err(StoreError::TooLarge { what: "the Markup strokes".into(), size, max: STROKES_MAX });
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| invalid(format!("the strokes are not JSON: {e}")))?;
    let top = v.as_object().ok_or_else(|| invalid("the strokes must be an object {\"strokes\": [...]}"))?;
    if let Some(k) = top.keys().find(|k| *k != "strokes") {
        return Err(invalid(format!("the strokes have an unknown key {k:?}")));
    }
    let strokes = top
        .get("strokes")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| invalid("\"strokes\" must be a list of one or more strokes"))?;
    for (i, s) in strokes.iter().enumerate() {
        let at = |reason: &str| invalid(format!("stroke {i}: {reason}"));
        let s = s.as_object().ok_or_else(|| at("not an object"))?;
        if let Some(k) = s.keys().find(|k| !matches!(k.as_str(), "color" | "width" | "tool" | "points")) {
            return Err(at(&format!("unknown key {k:?}")));
        }
        let color = s.get("color").and_then(Value::as_str).unwrap_or("");
        let hex = color.strip_prefix('#').unwrap_or("");
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(at("\"color\" must be #rrggbb or #rrggbbaa"));
        }
        match s.get("width").and_then(Value::as_f64) {
            Some(w) if w > 0.0 && w <= 1000.0 => {}
            _ => return Err(at("\"width\" must be a number above 0 and at most 1000")),
        }
        if let Some(tool) = s.get("tool") {
            let ok = tool
                .as_str()
                .is_some_and(|t| (1..=32).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'));
            if !ok {
                return Err(at("\"tool\" must be 1 to 32 of a-z and -"));
            }
        }
        let points =
            s.get("points").and_then(Value::as_array).filter(|p| !p.is_empty()).ok_or_else(|| at("no points"))?;
        for (j, p) in points.iter().enumerate() {
            let nums: Option<Vec<f64>> = p.as_array().and_then(|a| a.iter().map(Value::as_f64).collect());
            let ok = nums.is_some_and(|n| {
                (n.len() == 3 || n.len() == 4)
                    && n.iter().all(|x| x.is_finite())
                    && n[2] >= 0.0
                    && n.get(3).is_none_or(|f| *f >= 0.0)
            });
            if !ok {
                return Err(at(&format!("point {j} must be [x, y, t] or [x, y, t, force] (finite; t, force ≥ 0)")));
            }
        }
    }
    Ok(())
}

/// What a blob of a Markup is served as.
pub const IMAGE_TYPE: &str = "image/png";
pub const STROKES_TYPE: &str = "application/json";

impl Store {
    /// Check a Markup's image ([`check_png`]) and strokes ([`check_strokes`]) and store both as
    /// blobs for a live Document; nothing is attached yet, so no row and no event (as for a
    /// Version's blobs before its commit). A blob that was already stored gets a fresh grace
    /// period ([`crate::BlobStore::touch`]).
    pub fn upload_markup(&self, doc_id: &str, image: &[u8], strokes: &[u8]) -> Result<MarkupUpload> {
        {
            let conn = self.conn();
            let deleted: Option<Option<String>> =
                conn.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
            match deleted {
                None => return Err(StoreError::DocumentNotFound(doc_id.to_string())),
                Some(Some(_)) => return Err(StoreError::DocumentDeleted(doc_id.to_string())),
                Some(None) => {}
            }
        }
        let (width, height) = check_png(image)?;
        check_strokes(strokes)?;
        let (image_hash, made) = self.blobs.put(image)?;
        if !made {
            self.blobs.touch(&image_hash)?;
        }
        let (strokes_hash, made) = self.blobs.put(strokes)?;
        if !made {
            self.blobs.touch(&strokes_hash)?;
        }
        Ok(MarkupUpload {
            id: markup_id(&image_hash, &strokes_hash),
            markup: Markup {
                image: image_hash,
                width,
                height,
                image_size: image.len() as u64,
                strokes: strokes_hash,
                strokes_size: strokes.len() as u64,
            },
        })
    }

    /// The content type of `hash` when it is the image or the strokes of a Markup on a Comment
    /// of `doc_id` (what `GET /api/v1/docs/{id}/markups/{hash}` may serve); `None` for any other
    /// blob.
    pub fn markup_blob_type(&self, doc_id: &str, hash: &str) -> Result<Option<&'static str>> {
        let conn = self.conn();
        let found = conn
            .query_row(
                "SELECT 'image' FROM comment_markups WHERE doc_id = ?1 AND image_hash = ?2
                 UNION ALL SELECT 'strokes' FROM comment_markups WHERE doc_id = ?1 AND strokes_hash = ?2 LIMIT 1",
                params![doc_id, hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(found.map(|k| if k == "image" { IMAGE_TYPE } else { STROKES_TYPE }))
    }

    /// Attach Markup `id` to Comment `(thread_num, comment_num)` inside the Comment's own
    /// transaction: both blobs must be in the store (else `MarkupMissing`) and pass the checks
    /// again, whoever stored them.
    pub(crate) fn attach_markup(
        &self,
        tx: &Transaction,
        doc_id: &str,
        thread_num: u32,
        comment_num: u32,
        id: &str,
    ) -> Result<()> {
        let (image_hash, strokes_hash) = parse_markup_id(id)
            .ok_or_else(|| invalid(format!("{id:?} is not a Markup id (upload it: POST …/markups)")))?;
        let missing = || StoreError::MarkupMissing(id.trim().to_string());
        let image = self.blobs.read(image_hash)?.ok_or_else(missing)?;
        let strokes = self.blobs.read(strokes_hash)?.ok_or_else(missing)?;
        let (width, height) = check_png(&image)?;
        check_strokes(&strokes)?;
        tx.execute(
            "INSERT INTO comment_markups(doc_id, thread_num, comment_num, image_hash, image_size, width, height, strokes_hash, strokes_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                doc_id,
                thread_num as i64,
                comment_num as i64,
                image_hash,
                image.len() as i64,
                width as i64,
                height as i64,
                strokes_hash,
                strokes.len() as i64
            ],
        )?;
        Ok(())
    }
}

/// Every blob an attached Markup uses (the GC and `export` keep them: `docs::referenced_hashes`).
/// `export` reads stores older than v7 without migrating them, so a missing table is no Markups.
pub(crate) fn markup_blob_refs(tx: &rusqlite::Connection, doc_id: Option<&str>) -> Result<Vec<String>> {
    let has_table = tx
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'comment_markups'", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !has_table {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut stmt =
        tx.prepare("SELECT image_hash, strokes_hash FROM comment_markups WHERE ?1 IS NULL OR doc_id = ?1")?;
    for row in stmt.query_map([doc_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (i, s) = row?;
        out.push(i);
        out.push(s);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `w`×`h` RGBA PNG (white with a diagonal), for tests.
    pub fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let mut data = vec![255u8; (w * h * 4) as usize];
            for i in 0..w.min(h) {
                let p = ((i * w + i) * 4) as usize;
                data[p..p + 3].copy_from_slice(&[200, 30, 30]);
            }
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    pub const STROKES: &str =
        r##"{"strokes":[{"color":"#c81e1e","width":3,"tool":"pen","points":[[1,1,0],[5,5,0.02,0.5]]}]}"##;

    // spec: S49-01
    #[test]
    fn a_png_is_checked_by_decoding() {
        assert_eq!(check_png(&png_of(40, 20)).unwrap(), (40, 20));
        let err = |b: &[u8]| check_png(b).unwrap_err().to_string();
        assert!(err(b"GIF89a....").contains("not a PNG"));
        let mut cut = png_of(40, 20);
        cut.truncate(cut.len() - 20);
        assert!(err(&cut).contains("damaged"), "{}", err(&cut));
        let mut flipped = png_of(40, 20);
        let n = flipped.len();
        flipped[n - 30] ^= 0xff; // inside the image data: the checksum no longer holds
        assert!(err(&flipped).contains("damaged"));
        assert!(err(&png_of(4097, 1)).contains("4097×1"));
        let big = vec![0u8; IMAGE_MAX as usize + 1];
        assert!(matches!(check_png(&big), Err(StoreError::TooLarge { max: IMAGE_MAX, .. })));
    }

    // spec: S49-02
    #[test]
    fn strokes_have_one_shape() {
        check_strokes(STROKES.as_bytes()).unwrap();
        check_strokes(br##"{"strokes":[{"color":"#000000ff","width":0.5,"points":[[0,0,0]]}]}"##).unwrap();
        let bad = [
            ("[]", "an object"),
            ("{\"strokes\":[]}", "one or more"),
            ("{\"strokes\":[{\"color\":\"#000\",\"width\":1,\"points\":[[0,0,0]]}]}", "color"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":0,\"points\":[[0,0,0]]}]}", "width"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":1,\"points\":[]}]}", "no points"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":1,\"points\":[[0,0]]}]}", "point 0"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":1,\"points\":[[0,0,-1]]}]}", "point 0"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":1,\"tool\":\"Pen\",\"points\":[[0,0,0]]}]}", "tool"),
            ("{\"strokes\":[{\"color\":\"#000000\",\"width\":1,\"points\":[[0,0,0]],\"x\":1}]}", "unknown key"),
            ("{\"strokes\":[],\"v\":1}", "unknown key"),
            ("not json", "not JSON"),
        ];
        for (text, want) in bad {
            let e = check_strokes(text.as_bytes()).unwrap_err().to_string();
            assert!(e.contains(want), "{text}: {e}");
        }
        let big = vec![b' '; STROKES_MAX as usize + 1];
        assert!(matches!(check_strokes(&big), Err(StoreError::TooLarge { max: STROKES_MAX, .. })));
    }

    #[test]
    fn ids_name_both_blobs() {
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let id = markup_id(&a, &b);
        assert_eq!(parse_markup_id(&id), Some((a.as_str(), b.as_str())));
        assert_eq!(parse_markup_id("abc.def"), None);
        assert_eq!(parse_markup_id(&a), None);
    }
}
