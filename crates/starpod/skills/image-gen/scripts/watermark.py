"""Add text watermarks to images.

Usage:
    python watermark.py input.jpg "Confidential" [--output watermarked.jpg]
    python watermark.py photos/ "DRAFT" --opacity 0.3 --output marked/

Examples:
    python watermark.py photo.jpg "CONFIDENTIAL"
    python watermark.py report.png "DRAFT" --opacity 0.2 --angle 45
    python watermark.py images/ "© 2025" --position bottom-right --fontsize 24
"""

import argparse
import math
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    print("Error: Pillow required. Install: pip install Pillow", file=sys.stderr)
    sys.exit(1)


def add_watermark(
    input_path: str,
    text: str,
    output_path: str = None,
    opacity: float = 0.3,
    fontsize: int = None,
    position: str = "center",
    angle: float = 0,
    color: str = "white",
) -> str:
    img = Image.open(input_path).convert("RGBA")
    w, h = img.size

    if not fontsize:
        fontsize = max(16, min(w, h) // 15)

    # Try to load a good font
    font = None
    for font_path in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
    ]:
        try:
            font = ImageFont.truetype(font_path, fontsize)
            break
        except (OSError, IOError):
            continue
    if font is None:
        try:
            font = ImageFont.load_default(size=fontsize)
        except Exception:
            font = ImageFont.load_default()

    # Create watermark layer
    txt_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(txt_layer)

    # Parse color
    if color == "white":
        rgba = (255, 255, 255, int(255 * opacity))
    elif color == "black":
        rgba = (0, 0, 0, int(255 * opacity))
    else:
        # Hex color
        c = color.lstrip("#")
        r, g, b = int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)
        rgba = (r, g, b, int(255 * opacity))

    bbox = draw.textbbox((0, 0), text, font=font)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]

    # Position
    margin = max(20, min(w, h) // 30)
    positions = {
        "center": ((w - tw) // 2, (h - th) // 2),
        "top-left": (margin, margin),
        "top-right": (w - tw - margin, margin),
        "bottom-left": (margin, h - th - margin),
        "bottom-right": (w - tw - margin, h - th - margin),
    }
    pos = positions.get(position, positions["center"])
    draw.text(pos, text, fill=rgba, font=font)

    if angle:
        txt_layer = txt_layer.rotate(angle, center=(w // 2, h // 2), expand=False)

    result = Image.alpha_composite(img, txt_layer).convert("RGB")

    out = output_path or input_path
    result.save(out)
    return f"Watermarked: {input_path} -> {out}"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Add text watermarks to images")
    parser.add_argument("inputs", nargs="+", help="Input image(s) or directory")
    parser.add_argument("text", help="Watermark text")
    parser.add_argument("--output", "-o", help="Output file or directory")
    parser.add_argument("--opacity", type=float, default=0.3, help="Opacity 0-1 (default: 0.3)")
    parser.add_argument("--fontsize", type=int, help="Font size (auto if omitted)")
    parser.add_argument("--position", default="center",
                       choices=["center", "top-left", "top-right", "bottom-left", "bottom-right"])
    parser.add_argument("--angle", type=float, default=0, help="Rotation angle in degrees")
    parser.add_argument("--color", default="white", help="Text color: white, black, or hex (default: white)")
    args = parser.parse_args()

    # Separate text from inputs (last positional is text)
    inputs = args.inputs[:-1] if len(args.inputs) > 1 else args.inputs
    text = args.inputs[-1] if len(args.inputs) > 1 else args.text

    files = []
    for inp in inputs:
        p = Path(inp)
        if p.is_dir():
            files.extend(sorted(p.glob("*.jpg")) + sorted(p.glob("*.png")))
        elif p.exists():
            files.append(p)

    if not files:
        print("No image files found.", file=sys.stderr)
        sys.exit(1)

    out_dir = Path(args.output) if args.output and Path(args.output).suffix == "" else None
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)

    for f in files:
        out = str(out_dir / f.name) if out_dir else args.output
        msg = add_watermark(str(f), text, out, args.opacity, args.fontsize,
                           args.position, args.angle, args.color)
        print(f"  {msg}")
