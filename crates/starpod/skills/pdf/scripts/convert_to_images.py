"""Convert PDF pages to images for visual inspection.

Usage:
    python convert_to_images.py input.pdf [output_prefix] [--dpi N]

Examples:
    python convert_to_images.py document.pdf
    # Creates: page-01.jpg, page-02.jpg, ...

    python convert_to_images.py document.pdf slides --dpi 200
    # Creates: slides-01.jpg, slides-02.jpg, ...
"""

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path


def convert_pdf_to_images(
    pdf_path: str,
    output_prefix: str = "page",
    dpi: int = 150,
) -> list[str]:
    pdf = Path(pdf_path)
    if not pdf.exists():
        print(f"Error: {pdf_path} does not exist", file=sys.stderr)
        sys.exit(1)

    result = subprocess.run(
        ["pdftoppm", "-jpeg", "-r", str(dpi), str(pdf), output_prefix],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print(f"Error: pdftoppm failed: {result.stderr}", file=sys.stderr)
        print("Install poppler-utils: brew install poppler (macOS) or apt install poppler-utils (Linux)")
        sys.exit(1)

    images = sorted(Path(".").glob(f"{output_prefix}-*.jpg"))
    return [str(img) for img in images]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Convert PDF pages to JPEG images")
    parser.add_argument("input", help="Input PDF file")
    parser.add_argument("output_prefix", nargs="?", default="page", help="Output filename prefix (default: page)")
    parser.add_argument("--dpi", type=int, default=150, help="Resolution in DPI (default: 150)")
    args = parser.parse_args()

    images = convert_pdf_to_images(args.input, args.output_prefix, args.dpi)
    print(f"Created {len(images)} images:")
    for img in images:
        print(f"  {img}")
