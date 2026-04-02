"""Fill PDF form fields programmatically.

Usage:
    python fill_form.py input.pdf output.pdf --fields '{"field_name": "value", ...}'

Examples:
    python fill_form.py form.pdf filled.pdf --fields '{"Name": "John Doe", "Date": "2025-01-15"}'
    python fill_form.py form.pdf filled.pdf --json fields.json
"""

import argparse
import json
import sys
from pathlib import Path

try:
    from pypdf import PdfReader, PdfWriter
except ImportError:
    print("Error: pypdf required. Install: pip install pypdf", file=sys.stderr)
    sys.exit(1)


def list_fields(pdf_path: str) -> list[dict]:
    """List all fillable form fields in a PDF."""
    reader = PdfReader(pdf_path)
    fields = []

    if reader.get_fields():
        for name, field in reader.get_fields().items():
            fields.append({
                "name": name,
                "type": field.get("/FT", "unknown"),
                "value": field.get("/V", ""),
            })
    return fields


def fill_form(input_path: str, output_path: str, field_values: dict) -> None:
    reader = PdfReader(input_path)
    writer = PdfWriter()

    writer.append(reader)

    if not reader.get_fields():
        print("Warning: No fillable form fields found in PDF.", file=sys.stderr)
        print("Consider using annotation-based filling instead.", file=sys.stderr)

    writer.update_page_form_field_values(writer.pages[0], field_values)

    with open(output_path, "wb") as f:
        writer.write(f)

    print(f"Filled {len(field_values)} field(s) -> {output_path}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Fill PDF form fields")
    parser.add_argument("input", help="Input PDF file")
    parser.add_argument("output", nargs="?", help="Output PDF file")
    parser.add_argument("--fields", help="JSON string of field:value pairs")
    parser.add_argument("--json", help="JSON file with field:value pairs")
    parser.add_argument("--list", action="store_true", help="List available form fields")
    args = parser.parse_args()

    if args.list:
        fields = list_fields(args.input)
        if fields:
            print(f"Found {len(fields)} form field(s):")
            print(json.dumps(fields, indent=2))
        else:
            print("No fillable form fields found.")
        sys.exit(0)

    if not args.output:
        print("Error: output file required (unless using --list)", file=sys.stderr)
        sys.exit(1)

    if args.json:
        field_values = json.loads(Path(args.json).read_text())
    elif args.fields:
        field_values = json.loads(args.fields)
    else:
        print("Error: --fields or --json required", file=sys.stderr)
        sys.exit(1)

    fill_form(args.input, args.output, field_values)
