#!/usr/bin/env python3
"""Regenerates the static avatar fixtures (Member avatars): one 64x64 PNG face per animated SVG
in this directory, the shape the Hermes plugin uploads as the reduced-motion fallback. Pure
Python (zlib), no image library. Run from this directory: python3 make.py"""
import struct, zlib

def png(w, h, pixel):
    raw = b''.join(b'\x00' + b''.join(bytes(pixel(x, y)) for x in range(w)) for y in range(h))
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

def face(bg, skin):
    def pixel(x, y):
        cx, cy = x - 31.5, y - 31.5
        if cx * cx + cy * cy > 31 * 31:
            return (0, 0, 0, 0)
        for ex in (-7, 7):
            if (cx - ex) ** 2 + (cy + 2) ** 2 <= 9:
                return (30, 30, 30, 255)
        if 8 <= cy <= 10 and abs(cx) <= 8:
            return (30, 30, 30, 255)
        if (cx / 20) ** 2 + ((cy - 2) / 18) ** 2 <= 1:
            return skin + (255,)
        return bg + (255,)
    return pixel

open('scribe.png', 'wb').write(png(64, 64, face((60, 150, 90), (250, 220, 200))))
open('linter.png', 'wb').write(png(64, 64, face((60, 100, 190), (240, 210, 190))))
