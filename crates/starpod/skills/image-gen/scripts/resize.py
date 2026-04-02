"""Batch resize images with aspect ratio preservation.

Usage:
    python resize.py input.jpg --width 800
    python resize.py photos/ --width 1200 --height 630 --output resized/
    python resize.py *.png --max-size 500 --format webp

Examples:
    python resize.py photo.jpg --width 800
    python resize.py banner.png --width 1200 --height 630 --crop
    python resize.py images/ --max-size 500 --output thumbnails/
"""

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow required. Install: pip install Pillow", file=sys.stderr)
    sys.exit(1)


def resize_image(
    input_path: str,
    output_path: str = None,
    width: int = None,
    height: int = None,
    max_size: int = None,
    crop: bool = False,
    fmt: str = None,
    quality: int = 90,
) -> str:
    img = Image.open(input_path)
    orig_w, orig_h = img.size

    if max_size:
        img.thumbnail((max_size, max_size), Image.Resampling.LANCZOS)
    elif width and height and crop:
        # Center crop to exact dimensions
        target_ratio = width / height
        img_ratio = orig_w / orig_h
        if img_ratio > target_ratio:
            new_w = int(orig_h * target_ratio)
            left = (orig_w - new_w) // 2
            img = img.crop((left, 0, left + new_w, orig_h))
        else:
            new_h = int(orig_w / target_ratio)
            top = (orig_h - new_h) // 2
            img = img.crop((0, top, orig_w, top + new_h))
        img = img.resize((width, height), Image.Resampling.LANCZOS)
    elif width and height:
        img = img.resize((width, height), Image.Resampling.LANCZOS)
    elif width:
        ratio = width / orig_w
        img = img.resize((width, int(orig_h * ratio)), Image.Resampling.LANCZOS)
    elif height:
        ratio = height / orig_h
        img = img.resize((int(orig_w * ratio), height), Image.Resampling.LANCZOS)

    out = Path(output_path or input_path)
    if fmt:
        out = out.with_suffix(f".{fmt}")

    save_kwargs = {}
    if out.suffix.lower() in (".jpg", ".jpeg"):
        save_kwargs["quality"] = quality
        if img.mode == "RGBA":
            img = img.convert("RGB")
    elif out.suffix.lower() == ".webp":
        save_kwargs["quality"] = quality

    img.save(out, **save_kwargs)
    return f"{input_path} ({orig_w}x{orig_h}) -> {out} ({img.size[0]}x{img.size[1]})"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Resize images")
    parser.add_argument("inputs", nargs="+", help="Input image(s) or directory")
    parser.add_argument("--width", "-w", type=int, help="Target width")
    parser.add_argument("--height", type=int, help="Target height")
    parser.add_argument("--max-size", type=int, help="Max dimension (preserves aspect ratio)")
    parser.add_argument("--crop", action="store_true", help="Center-crop to exact dimensions")
    parser.add_argument("--output", "-o", help="Output directory")
    parser.add_argument("--format", "-f", help="Output format (jpg, png, webp)")
    parser.add_argument("--quality", "-q", type=int, default=90, help="JPEG/WebP quality (default: 90)")
    args = parser.parse_args()

    if not (args.width or args.height or args.max_size):
        print("Error: Specify --width, --height, or --max-size", file=sys.stderr)
        sys.exit(1)

    files = []
    for inp in args.inputs:
        p = Path(inp)
        if p.is_dir():
            files.extend(sorted(p.glob("*.jpg")) + sorted(p.glob("*.png")) + sorted(p.glob("*.webp")))
        elif p.exists():
            files.append(p)

    if not files:
        print("No image files found.", file=sys.stderr)
        sys.exit(1)

    out_dir = Path(args.output) if args.output else None
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)

    for f in files:
        out = str(out_dir / f.name) if out_dir else None
        msg = resize_image(str(f), out, args.width, args.height, args.max_size,
                          args.crop, args.format, args.quality)
        print(f"  {msg}")

    print(f"Resized {len(files)} image(s)")
