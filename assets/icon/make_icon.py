"""Render the CANoxide app icon: PNGs, a Windows .ico and a preview sheet.

Offscreen only (PyMuPDF renders the SVG, Pillow writes the images); no window opens.
Run from anywhere:  py -3 make_icon.py
Needs: PyMuPDF (fitz), Pillow.
"""
import os
import struct
import sys
from io import BytesIO

import fitz
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
MASTER = os.path.join(HERE, "canoxide.svg")
SMALL = os.path.join(HERE, "canoxide_small.svg")

SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SMALL_MAX = int(sys.argv[1]) if len(sys.argv) > 1 else 24   # sizes up to this use the small design


def render(svg_path, size):
    doc = fitz.open(svg_path)
    page = doc[0]
    pix = page.get_pixmap(matrix=fitz.Matrix(size / page.rect.width, size / page.rect.height), alpha=True)
    img = Image.frombytes("RGBA", (pix.width, pix.height), pix.samples)
    doc.close()
    return img


def bmp_entry(img):
    """32-bit BGRA DIB for an ICO entry: header with double height, bottom-up pixels, empty AND mask."""
    w, h = img.size
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = img.tobytes("raw", "BGRA", 0, -1)          # bottom-up rows
    mask_row = ((w + 31) // 32) * 4
    return header + px + b"\x00" * (mask_row * h)


def write_ico(path, images):
    entries = []
    for img in images:
        if img.width >= 256:
            buf = BytesIO()
            img.save(buf, "PNG", optimize=True)
            entries.append((img.width, buf.getvalue()))
        else:
            entries.append((img.width, bmp_entry(img)))
    out = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    data = b""
    for size, blob in entries:
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        data += blob
    with open(path, "wb") as fh:
        fh.write(out + data)


def read_ico(path):
    with open(path, "rb") as fh:
        raw = fh.read()
    _, typ, count = struct.unpack_from("<HHH", raw, 0)
    result = []
    for i in range(count):
        w, h, _, _, _, bpp, size, off = struct.unpack_from("<BBBBHHII", raw, 6 + 16 * i)
        kind = "PNG" if raw[off:off + 8] == b"\x89PNG\r\n\x1a\n" else "BMP"
        result.append((w or 256, h or 256, bpp, kind, size))
    return typ, result


def main():
    images = []
    for n in SIZES:
        img = render(SMALL if n <= SMALL_MAX else MASTER, n)
        img.save(os.path.join(HERE, f"canoxide_{n}.png"), optimize=True)
        images.append(img)
    ico = os.path.join(HERE, "canoxide.ico")
    write_ico(ico, images)
    typ, entries = read_ico(ico)
    print(f"canoxide.ico: type {typ}, {len(entries)} entries")
    for e in entries:
        print("  %dx%d %d-bit %s %d bytes" % e)
    with Image.open(ico) as check:      # second opinion: Pillow reads the file back
        print("  Pillow sizes:", sorted(check.info.get("sizes", [])))

    # preview: every size on dark and light taskbar-like strips, 16 px at 8x
    try:
        font = ImageFont.truetype("segoeui.ttf", 13)
    except OSError:
        font = ImageFont.load_default()
    pad, zoom = 20, 8
    width = pad + sum(n + pad for n in SIZES) + 16 * zoom + pad
    strip_h = 256 + 2 * pad + 20
    sheet = Image.new("RGB", (width, 2 * strip_h), (0, 0, 0))
    d = ImageDraw.Draw(sheet)
    for k, (bg, fg) in enumerate((((32, 32, 32), (170, 170, 170)), ((243, 243, 243), (80, 80, 80)))):
        y0 = k * strip_h
        d.rectangle([0, y0, width, y0 + strip_h], fill=bg)
        x = pad
        for n, img in zip(SIZES, images):
            sheet.paste(img, (x, y0 + pad + (256 - n) // 2), img)
            d.text((x, y0 + pad + 256 + 2), str(n), font=font, fill=fg)
            x += n + pad
        z = images[0].resize((16 * zoom, 16 * zoom), Image.NEAREST)
        sheet.paste(z, (x, y0 + pad + (256 - 16 * zoom) // 2), z)
        d.text((x, y0 + pad + 256 + 2), "16 at 8x", font=font, fill=fg)
    sheet.save(os.path.join(HERE, "preview.png"), optimize=True)


if __name__ == "__main__":
    main()
