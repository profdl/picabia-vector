//! Tests for a screen's image tone read through the document (PIC20.5d): headers checked before
//! decoding, failures remembered, many images kept by content.

use std::sync::atomic::Ordering;

use vectorcraft_doc::{Appearance, Document, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use crate::target::DECODES;
use crate::*;

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

fn ihdr(w: u32, h: u32) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend(w.to_be_bytes());
    d.extend(h.to_be_bytes());
    d.extend([8, 0, 0, 0, 0]);
    d
}

/// An 8-bit grey PNG (written here: stored deflate blocks), `grey(x, y)` per pixel.
fn png(w: u32, h: u32, grey: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        raw.extend((0..w).map(|x| grey(x, y)));
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (k, b) in blocks.iter().enumerate() {
        z.push(u8::from(k + 1 == blocks.len()));
        let len = b.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(*b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr(w, h));
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A document holding `images` (each placed on a 100 pt square) and a square to screen.
fn doc_with(images: &[Vec<u8>], claimed: (u32, u32)) -> (Document, NodeId, Vec<NodeId>) {
    let mut d = Document::new(612.0, 792.0);
    let l = d.layers[0].id;
    let mut ids = Vec::new();
    for (k, bytes) in images.iter().enumerate() {
        let key = format!("img{k}");
        d.images.insert(key.clone(), ImageBlob::new("image/png", bytes.clone()));
        let id = d.alloc_id();
        let im = ImageObject {
            key,
            width: claimed.0,
            height: claimed.1,
            xf: Affine::scale_non_uniform(100.0 / f64::from(claimed.0.max(1)), 100.0 / f64::from(claimed.1.max(1))),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        ids.push(d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap());
    }
    let sq = d.alloc_id();
    let sq = d.insert(Some(l), 0, Node::path(sq, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art())).unwrap();
    (d, sq, ids)
}

/// One test: the decode counter is shared by every image decode in this crate's tests.
#[test]
fn image_headers_are_checked_first_failures_are_remembered_and_many_images_kept() {
    // A ~70-byte file claiming 22 000 × 22 000 pixels, its data cut off: refused from its header,
    // nothing decoded, and the refusal remembered (no second attempt).
    let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut huge, b"IHDR", &ihdr(22_000, 22_000));
    huge.extend(b"\x00\x00\x00\x10IDAT\x78\x01");
    let (d, sq, ids) = doc_with(&[huge], (22_000, 22_000));
    let t = DocTargets::new(&d, sq);
    let before = DECODES.load(Ordering::Relaxed);
    let started = std::time::Instant::now();
    for _ in 0..3 {
        let e = t.tone(&ids[0].0.to_string()).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }
    assert!(started.elapsed().as_secs_f64() < 1.0, "{:?}", started.elapsed());
    assert_eq!(DECODES.load(Ordering::Relaxed) - before, 1, "one attempt, then remembered");
    // A truncated small file: fails to decode once, remembered.
    let mut cut = png(8, 8, |x, _| (x * 30) as u8);
    cut.truncate(cut.len() - 20);
    let (d, sq, ids) = doc_with(&[cut], (8, 8));
    let t = DocTargets::new(&d, sq);
    let before = DECODES.load(Ordering::Relaxed);
    for _ in 0..3 {
        assert!(t.tone(&ids[0].0.to_string()).is_err());
    }
    assert_eq!(DECODES.load(Ordering::Relaxed) - before, 1);
    // Six distinct images: each decoded once, then kept (by content), however often they are read.
    let images: Vec<Vec<u8>> = (0..6u8).map(|k| png(16, 16, move |x, _| if x < 8 { k * 20 } else { 255 })).collect();
    let (d, sq, ids) = doc_with(&images, (16, 16));
    let t = DocTargets::new(&d, sq);
    let before = DECODES.load(Ordering::Relaxed);
    for _ in 0..4 {
        for id in &ids {
            let tone = t.tone(&id.0.to_string()).unwrap();
            assert_eq!(tone.image.bytes(), 256, "one byte a pixel");
            assert!(tone.darkness_at(Point::new(90.0, 50.0)) < 0.01, "white on the right");
        }
    }
    assert_eq!(DECODES.load(Ordering::Relaxed) - before, 6);
    // The same pixels under another key share the decode.
    let (d, sq, ids) = doc_with(&images[..1], (16, 16));
    let before = DECODES.load(Ordering::Relaxed);
    assert!(DocTargets::new(&d, sq).tone(&ids[0].0.to_string()).is_ok());
    assert_eq!(DECODES.load(Ordering::Relaxed) - before, 0, "kept by content");
}
