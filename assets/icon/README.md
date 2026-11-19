# CANoxide app icon

The mark is an "Ox" monogram on a navy tile. The "O" is a green ring. The "x" is a
twisted wire pair: a light-grey wire passes over a mid-grey wire, which is cut
where the two cross.

## Files

| File | What it is |
|---|---|
| `canoxide.svg` | Master design (256 x 256 view box). Used for 32 px and above. |
| `canoxide_small.svg` | Simplified design for 16 to 24 px: thicker strokes, edge on the pixel grid, x without the cut, x wider. |
| `canoxide_<n>.png` | PNG at n = 16, 20, 24, 32, 40, 48, 64, 128, 256 px, transparent outside the tile. 16, 20 and 24 come from `canoxide_small.svg`, the rest from `canoxide.svg`. |
| `canoxide.ico` | Windows icon with all nine sizes: 256 px as a PNG entry, the others as 32-bit BMP entries with alpha. |
| `preview.png` | All sizes on a dark and a light taskbar-like strip, plus 16 px at 8x (nearest neighbour). |
| `make_icon.py` | Generator for the PNG, ICO and preview files. |

## Colours

| Part | Hex |
|---|---|
| Tile fill | `#16202B` |
| Tile edge | `#2E3B4A` |
| "O" ring | `#3DAE49` |
| "x" over-wire | `#E6E9ED` |
| "x" under-wire | `#8C96A3` |

Each colour has a contrast of at least 3:1 against the tile fill. The lowest is the under-wire, at about 5.5:1.

## Generating the files

```
py -3 make_icon.py
```

The script needs PyMuPDF (`fitz`) and Pillow. It renders the SVGs offscreen and opens no window. It writes
every PNG, `canoxide.ico` and `preview.png` next to itself, and it then reads the ICO back and prints the
entries. An optional argument sets the largest size that uses the small design (default 24).
Edit the SVGs by hand and run the script again. Do not edit the PNG or ICO files directly.
