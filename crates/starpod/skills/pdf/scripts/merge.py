"""Merge multiple PDF files into one.

Usage:
    python merge.py output.pdf input1.pdf input2.pdf [input3.pdf ...]

Examples:
    python merge.py combined.pdf chapter1.pdf chapter2.pdf chapter3.pdf
"""

import argparse
import sys
from pathlib import Path

try:
    from pypdf import PdfWriter, PdfReader
except ImportError:
    print("Error: pypdf required. Install: pip install pypdf", file=sys.stderr)
    sys.exit(1)


def merge_pdfs(output_path: str, input_paths: list[str]) -> None:
    writer = PdfWriter()
    total_pages = 0

    for pdf_path in input_paths:
        path = Path(pdf_path)
        if not path.exists():
            print(f"Error: {pdf_path} does not exist", file=sys.stderr)
            sys.exit(1)

        reader = PdfReader(path)
        for page in reader.pages:
            writer.add_page(page)
        total_pages += len(reader.pages)
        print(f"  Added: {pdf_path} ({len(reader.pages)} pages)")

    with open(output_path, "wb") as f:
        writer.write(f)

    print(f"Merged {len(input_paths)} files ({total_pages} pages) -> {output_path}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Merge multiple PDFs into one")
    parser.add_argument("output", help="Output PDF file")
    parser.add_argument("inputs", nargs="+", help="Input PDF files to merge")
    args = parser.parse_args()
    merge_pdfs(args.output, args.inputs)
