"""Unpack Office files (DOCX, PPTX, XLSX) for XML editing.

Extracts the ZIP archive, pretty-prints XML files, and escapes smart quotes
so they survive round-trip editing.

Usage:
    python unpack.py document.docx unpacked/
    python unpack.py presentation.pptx unpacked/
"""

import argparse
import sys
import zipfile
from pathlib import Path
from xml.dom import minidom

SMART_QUOTE_REPLACEMENTS = {
    "\u201c": "&#x201C;",
    "\u201d": "&#x201D;",
    "\u2018": "&#x2018;",
    "\u2019": "&#x2019;",
}


def unpack(input_file: str, output_directory: str) -> str:
    input_path = Path(input_file)
    output_path = Path(output_directory)

    if not input_path.exists():
        return f"Error: {input_file} does not exist"

    if input_path.suffix.lower() not in {".docx", ".pptx", ".xlsx"}:
        return f"Error: {input_file} must be a .docx, .pptx, or .xlsx file"

    try:
        output_path.mkdir(parents=True, exist_ok=True)

        with zipfile.ZipFile(input_path, "r") as zf:
            zf.extractall(output_path)

        xml_files = list(output_path.rglob("*.xml")) + list(output_path.rglob("*.rels"))
        for xml_file in xml_files:
            _pretty_print_xml(xml_file)

        for xml_file in xml_files:
            _escape_smart_quotes(xml_file)

        return f"Unpacked {input_file} ({len(xml_files)} XML files) to {output_directory}"

    except zipfile.BadZipFile:
        return f"Error: {input_file} is not a valid Office file"
    except Exception as e:
        return f"Error unpacking: {e}"


def _pretty_print_xml(xml_file: Path) -> None:
    try:
        content = xml_file.read_text(encoding="utf-8")
        dom = minidom.parseString(content)
        xml_file.write_bytes(dom.toprettyxml(indent="  ", encoding="utf-8"))
    except Exception:
        pass


def _escape_smart_quotes(xml_file: Path) -> None:
    try:
        content = xml_file.read_text(encoding="utf-8")
        for char, entity in SMART_QUOTE_REPLACEMENTS.items():
            content = content.replace(char, entity)
        xml_file.write_text(content, encoding="utf-8")
    except Exception:
        pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Unpack an Office file for editing")
    parser.add_argument("input_file", help="Office file to unpack")
    parser.add_argument("output_directory", help="Output directory")
    args = parser.parse_args()

    message = unpack(args.input_file, args.output_directory)
    print(message)
    if "Error" in message:
        sys.exit(1)
