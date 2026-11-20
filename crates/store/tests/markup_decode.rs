//! `check_png` against hand-built PNGs, measuring what the decoder allocates on the checking
//! thread. Its own test binary: the counting allocator is process-wide (the counters are not).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use annotary_store::markup::{MAX_SIDE, check_png};

struct Counting;

thread_local! {
    static NOW: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

fn grew(n: usize) {
    let _ = NOW.try_with(|now| {
        let v = now.get() + n;
        now.set(v);
        let _ = PEAK.try_with(|p| p.set(p.get().max(v)));
    });
}

fn shrank(n: usize) {
    let _ = NOW.try_with(|now| now.set(now.get().saturating_sub(n)));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grew(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        shrank(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        shrank(l.size());
        grew(new);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// `check_png(bytes)` and the most bytes this thread held on the heap above its start meanwhile.
fn checked(bytes: &[u8]) -> (Result<(u32, u32), String>, usize) {
    let base = NOW.with(Cell::get);
    PEAK.with(|p| p.set(base));
    let out = check_png(bytes).map_err(|e| e.to_string());
    (out, PEAK.with(Cell::get) - base)
}

const MIB: usize = 1024 * 1024;

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = !0u32;
    for b in parts.iter().flat_map(|p| p.iter()) {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for x in data {
        a = (a + u32::from(*x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// `data` as a zlib stream of stored (uncompressed) blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65535).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let n = b.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(!n).to_le_bytes());
        out.extend_from_slice(b);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// A zlib stream that inflates to `len` (1 or more) zero bytes: one fixed-Huffman block of up to
/// 258 literal zeros, then copies of length 258 at distance 1 (13 bits each).
fn zlib_zeros(len: usize) -> Vec<u8> {
    struct Bits {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }
    impl Bits {
        fn put(&mut self, v: u64, n: u32) {
            self.acc |= v << self.n;
            self.n += n;
            while self.n >= 8 {
                self.out.push(self.acc as u8);
                self.acc >>= 8;
                self.n -= 8;
            }
        }
        /// A Huffman code goes in most significant bit first.
        fn code(&mut self, v: u64, n: u32) {
            self.put(v.reverse_bits() >> (64 - n), n);
        }
    }
    let mut w = Bits { out: vec![0x78, 0x01], acc: 0, n: 0 };
    w.put(1, 1); // BFINAL
    w.put(1, 2); // BTYPE 01, fixed Huffman
    let literals = 1 + (len - 1) % 258;
    for _ in 0..literals {
        w.code(0b0011_0000, 8); // literal 0
    }
    for _ in 0..(len - literals) / 258 {
        w.code(0b1100_0101, 8); // length 258 (code 285)
        w.code(0, 5); // distance 1
    }
    w.code(0, 7); // end of block
    if w.n > 0 {
        w.put(0, 8 - w.n);
    }
    let b = (len % 65521) as u32;
    w.out.extend_from_slice(&((b << 16) | 1).to_be_bytes());
    w.out
}

struct Png {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlaced: bool,
    plte: Option<Vec<u8>>,
    iccp: Option<(&'static str, Vec<u8>)>,
}

impl Png {
    fn new(width: u32, height: u32, depth: u8, color: u8) -> Self {
        Png { width, height, depth, color, interlaced: false, plte: None, iccp: None }
    }

    fn row_bytes(&self, w: u32) -> usize {
        let channels = match self.color {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            _ => 4,
        };
        (w as usize * channels * self.depth as usize).div_ceil(8)
    }

    /// The length of the filtered image data (a filter byte per row).
    fn raw_len(&self) -> usize {
        let passes: &[(u32, u32, u32, u32)] = if self.interlaced {
            &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
        } else {
            &[(0, 0, 1, 1)]
        };
        let mut n = 0;
        for &(x0, y0, dx, dy) in passes {
            let w = self.width.saturating_sub(x0).div_ceil(dx);
            let h = self.height.saturating_sub(y0).div_ceil(dy);
            if w > 0 && h > 0 {
                n += h as usize * (1 + self.row_bytes(w));
            }
        }
        n
    }

    /// The PNG with every pixel zero (`idat` stored, or one run of zeros for a large image).
    fn build(&self) -> Vec<u8> {
        let raw = self.raw_len();
        let idat = if raw > 4 * MIB { zlib_zeros(raw) } else { zlib_stored(&vec![0; raw]) };
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&self.width.to_be_bytes());
        ihdr.extend_from_slice(&self.height.to_be_bytes());
        ihdr.extend_from_slice(&[self.depth, self.color, 0, 0, u8::from(self.interlaced)]);
        chunk(&mut out, b"IHDR", &ihdr);
        if let Some((name, z)) = &self.iccp {
            let mut data = name.as_bytes().to_vec();
            data.extend_from_slice(&[0, 0]);
            data.extend_from_slice(z);
            chunk(&mut out, b"iCCP", &data);
        }
        if let Some(p) = &self.plte {
            chunk(&mut out, b"PLTE", p);
        }
        chunk(&mut out, b"IDAT", &idat);
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

// spec: S49-01
#[test]
fn an_iccp_bomb_is_not_inflated() {
    // 128 MiB of "profile" in about 830 KiB.
    let mut bomb = Png::new(8, 8, 8, 6);
    bomb.iccp = Some(("Display P3", zlib_zeros(128 * MIB)));
    let bytes = bomb.build();
    assert!(bytes.len() < 2 * MIB, "{}", bytes.len());
    let (out, peak) = checked(&bytes);
    assert_eq!(out, Ok((8, 8)));
    assert!(peak < 2 * MIB, "checking the PNG held {peak} bytes");
}

// spec: S49-01
#[test]
fn the_largest_markup_decodes_without_its_frame() {
    // 4,096² in 16-bit RGBA is 128 MiB of pixels.
    let big = Png::new(MAX_SIDE, MAX_SIDE, 16, 6).build();
    assert!(big.len() < 2 * MIB, "{}", big.len());
    let (out, peak) = checked(&big);
    assert_eq!(out, Ok((MAX_SIDE, MAX_SIDE)));
    assert!(peak < 2 * MIB, "checking the PNG held {peak} bytes");
}

// spec: S49-01
#[test]
fn interlaced_palette_and_profiled_pngs_pass() {
    let mut adam7 = Png::new(333, 77, 16, 6);
    adam7.interlaced = true;
    assert_eq!(checked(&adam7.build()).0, Ok((333, 77)));

    let mut palette = Png::new(50, 30, 8, 3);
    palette.plte = Some(vec![255, 255, 255, 200, 30, 30]);
    assert_eq!(checked(&palette.build()).0, Ok((50, 30)));
    let mut low = Png::new(17, 9, 2, 3);
    low.plte = Some(vec![0; 12]);
    low.interlaced = true;
    assert_eq!(checked(&low.build()).0, Ok((17, 9)));

    for name in ["Display P3", "sRGB"] {
        let mut p = Png::new(40, 20, 8, 6);
        p.iccp = Some((name, zlib_stored(&[7; 3144])));
        assert_eq!(checked(&p.build()).0, Ok((40, 20)), "{name}");
    }
    // A damaged profile is not read either: only the pixels are checked.
    let mut broken = Png::new(40, 20, 8, 6);
    broken.iccp = Some(("Display P3", b"not zlib".to_vec()));
    assert_eq!(checked(&broken.build()).0, Ok((40, 20)));
}

// spec: S49-01
#[test]
fn dimensions_are_refused_before_the_metadata_is_read() {
    // Too large a side, behind a bomb that would inflate if the metadata were read first.
    let mut wide = Png::new(MAX_SIDE + 1, 1, 8, 6);
    wide.iccp = Some(("x", zlib_zeros(128 * MIB)));
    // Only the header is read, so the image data's length need not match.
    let mut bytes = wide.build();
    bytes.truncate(8 + 25 + 1024);
    let (out, peak) = checked(&bytes);
    assert!(out.unwrap_err().contains("4097×1"));
    assert!(peak < MIB, "{peak}");
}
