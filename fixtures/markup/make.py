#!/usr/bin/env python3
"""Regenerates the Markup fixtures: `cross-out.png`, a 480x160 crop of a page in the light theme
(gray bars for three lines of text) with red ink striking the middle line and a loop in the
margin, and `cross-out.strokes.json`, the same two strokes in the PNG's pixel coordinates (the
shape `annotary_store::markup::check_strokes` accepts). Pure Python (zlib), no image library.
Run from this directory: python3 make.py"""
import json, math, struct, zlib

W, H = 480, 160
INK = (200, 30, 30)

strike = [[40 + i * 10, 80 + (i % 2), round(i * 0.012, 3)] for i in range(33)]
loop = [[430 + round(22 * math.cos(a / 10 * math.pi), 1), 80 + round(22 * math.sin(a / 10 * math.pi), 1),
         round(0.6 + a * 0.015, 3), 0.6] for a in range(21)]
strokes = {"strokes": [
    {"color": "#c81e1e", "width": 4, "tool": "pen", "points": strike},
    {"color": "#c81e1e", "width": 3, "tool": "pen", "points": loop},
]}

def near(x, y, pts, r):
    for (ax, ay, *_), (bx, by, *_) in zip(pts, pts[1:]):
        dx, dy = bx - ax, by - ay
        t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / ((dx * dx + dy * dy) or 1)))
        if (x - ax - t * dx) ** 2 + (y - ay - t * dy) ** 2 <= r * r:
            return True
    return False

def pixel(x, y):
    if near(x, y, strike, 2) or near(x, y, loop, 1.5):
        return INK + (255,)
    for top in (40, 74, 108):
        if top <= y < top + 12 and 30 <= x < 400:
            return (120, 120, 120, 255)
    return (255, 255, 255, 255)

def png(w, h):
    raw = b''.join(b'\x00' + b''.join(bytes(pixel(x, y)) for x in range(w)) for y in range(h))
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

open('cross-out.png', 'wb').write(png(W, H))
open('cross-out.strokes.json', 'w').write(json.dumps(strokes, separators=(',', ':')) + '\n')
