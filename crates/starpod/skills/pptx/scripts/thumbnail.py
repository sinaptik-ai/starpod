"""Create thumbnail grids from PowerPoint slides for visual QA.

Labels each thumbnail with its slide filename. Hidden slides shown
with a placeholder pattern.

Usage:
    python thumbnail.py presentation.pptx [output_prefix] [--cols N]

Examples:
    python thumbnail.py deck.pptx                    # -> thumbnails.jpg
    python thumbnail.py deck.pptx grid --cols 4      # -> grid.jpg
"""

import argparse
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from xml.dom import minidom

try:
    from office.soffice import get_soffice_env
except ImportError:
    def get_soffice_env():
        import os
        return os.environ.copy()

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    print("Error: Pillow required. Install: pip install Pillow", file=sys.stderr)
    sys.exit(1)

THUMB_WIDTH = 300
DPI = 100
PADDING = 20
BORDER = 2
JPEG_QUALITY = 95


def get_slide_info(pptx_path: Path) -> list[dict]:
    """Get ordered list of slides with hidden status."""
    with zipfile.ZipFile(pptx_path, "r") as zf:
        rels = minidom.parseString(zf.read("ppt/_rels/presentation.xml.rels").decode())
        rid_to_slide = {}
        for rel in rels.getElementsByTagName("Relationship"):
            target = rel.getAttribute("Target")
            if "slide" in rel.getAttribute("Type") and target.startswith("slides/"):
                rid_to_slide[rel.getAttribute("Id")] = target.replace("slides/", "")

        pres = minidom.parseString(zf.read("ppt/presentation.xml").decode())
        slides = []
        for sld_id in pres.getElementsByTagName("p:sldId"):
            rid = sld_id.getAttribute("r:id")
            if rid in rid_to_slide:
                slides.append({
                    "name": rid_to_slide[rid],
                    "hidden": sld_id.getAttribute("show") == "0",
                })
    return slides


def convert_to_images(pptx_path: Path, temp_dir: Path) -> list[Path]:
    """Convert PPTX to slide images via LibreOffice + pdftoppm."""
    pdf_path = temp_dir / f"{pptx_path.stem}.pdf"

    result = subprocess.run(
        ["soffice", "--headless", "--convert-to", "pdf", "--outdir", str(temp_dir), str(pptx_path)],
        capture_output=True, text=True, env=get_soffice_env(),
    )
    if result.returncode != 0 or not pdf_path.exists():
        raise RuntimeError(f"PDF conversion failed: {result.stderr}")

    result = subprocess.run(
        ["pdftoppm", "-jpeg", "-r", str(DPI), str(pdf_path), str(temp_dir / "slide")],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"Image conversion failed: {result.stderr}")

    return sorted(temp_dir.glob("slide-*.jpg"))


def create_grid(slides: list[tuple[Path, str]], cols: int, width: int) -> Image.Image:
    """Create a grid of slide thumbnails with labels."""
    font_size = int(width * 0.1)
    label_pad = int(font_size * 0.4)

    with Image.open(slides[0][0]) as img:
        aspect = img.height / img.width
    height = int(width * aspect)

    rows = (len(slides) + cols - 1) // cols
    grid_w = cols * width + (cols + 1) * PADDING
    grid_h = rows * (height + font_size + label_pad * 2) + (rows + 1) * PADDING

    grid = Image.new("RGB", (grid_w, grid_h), "white")
    draw = ImageDraw.Draw(grid)

    try:
        font = ImageFont.load_default(size=font_size)
    except Exception:
        font = ImageFont.load_default()

    for i, (img_path, label) in enumerate(slides):
        row, col = divmod(i, cols)
        x = col * width + (col + 1) * PADDING
        y_base = row * (height + font_size + label_pad * 2) + (row + 1) * PADDING

        bbox = draw.textbbox((0, 0), label, font=font)
        tw = bbox[2] - bbox[0]
        draw.text((x + (width - tw) // 2, y_base + label_pad), label, fill="black", font=font)

        y_thumb = y_base + label_pad + font_size + label_pad
        with Image.open(img_path) as img:
            img.thumbnail((width, height), Image.Resampling.LANCZOS)
            w, h = img.size
            tx, ty = x + (width - w) // 2, y_thumb + (height - h) // 2
            grid.paste(img, (tx, ty))
            draw.rectangle(
                [(tx - BORDER, ty - BORDER), (tx + w + BORDER - 1, ty + h + BORDER - 1)],
                outline="gray", width=BORDER,
            )

    return grid


def main():
    parser = argparse.ArgumentParser(description="Create thumbnail grids from PPTX slides")
    parser.add_argument("input", help="Input PowerPoint file")
    parser.add_argument("output_prefix", nargs="?", default="thumbnails", help="Output prefix (default: thumbnails)")
    parser.add_argument("--cols", type=int, default=3, help="Columns (default: 3, max: 6)")
    args = parser.parse_args()

    cols = min(args.cols, 6)
    input_path = Path(args.input)

    if not input_path.exists() or input_path.suffix.lower() != ".pptx":
        print(f"Error: Invalid PowerPoint file: {args.input}", file=sys.stderr)
        sys.exit(1)

    slide_info = get_slide_info(input_path)

    with tempfile.TemporaryDirectory() as temp_dir:
        temp_path = Path(temp_dir)
        visible_images = convert_to_images(input_path, temp_path)

        slides = []
        vis_idx = 0
        for info in slide_info:
            if info["hidden"]:
                # Create hidden placeholder
                if visible_images:
                    with Image.open(visible_images[0]) as img:
                        size = img.size
                else:
                    size = (1920, 1080)
                ph = Image.new("RGB", size, "#F0F0F0")
                d = ImageDraw.Draw(ph)
                lw = max(5, min(size) // 100)
                d.line([(0, 0), size], fill="#CCCCCC", width=lw)
                d.line([(size[0], 0), (0, size[1])], fill="#CCCCCC", width=lw)
                ph_path = temp_path / f"hidden-{info['name']}.jpg"
                ph.save(ph_path, "JPEG")
                slides.append((ph_path, f"{info['name']} (hidden)"))
            else:
                if vis_idx < len(visible_images):
                    slides.append((visible_images[vis_idx], info["name"]))
                    vis_idx += 1

        if not slides:
            print("Error: No slides found", file=sys.stderr)
            sys.exit(1)

        grid = create_grid(slides, cols, THUMB_WIDTH)
        output_file = f"{args.output_prefix}.jpg"
        grid.save(output_file, quality=JPEG_QUALITY)
        print(f"Created: {output_file} ({len(slides)} slides, {cols} cols)")


if __name__ == "__main__":
    main()
