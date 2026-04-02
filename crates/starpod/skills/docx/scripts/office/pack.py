"""Pack an unpacked directory back into a DOCX, PPTX, or XLSX file.

Condenses XML formatting and creates the Office ZIP archive.

Usage:
    python pack.py unpacked/ output.docx
    python pack.py unpacked/ output.pptx
"""

import argparse
import shutil
import sys
import tempfile
import zipfile
from pathlib import Path
from xml.dom import minidom


def pack(input_directory: str, output_file: str) -> str:
    input_dir = Path(input_directory)
    output_path = Path(output_file)

    if not input_dir.is_dir():
        return f"Error: {input_dir} is not a directory"

    if output_path.suffix.lower() not in {".docx", ".pptx", ".xlsx"}:
        return f"Error: {output_file} must be a .docx, .pptx, or .xlsx file"

    with tempfile.TemporaryDirectory() as temp_dir:
        temp_content_dir = Path(temp_dir) / "content"
        shutil.copytree(input_dir, temp_content_dir)

        for pattern in ["*.xml", "*.rels"]:
            for xml_file in temp_content_dir.rglob(pattern):
                _condense_xml(xml_file)

        output_path.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(output_path, "w", zipfile.ZIP_DEFLATED) as zf:
            for f in sorted(temp_content_dir.rglob("*")):
                if f.is_file():
                    zf.write(f, f.relative_to(temp_content_dir))

    return f"Successfully packed {input_dir} to {output_file}"


def _condense_xml(xml_file: Path) -> None:
    try:
        with open(xml_file, encoding="utf-8") as f:
            dom = minidom.parse(f)

        for element in dom.getElementsByTagName("*"):
            if element.tagName.endswith(":t"):
                continue
            for child in list(element.childNodes):
                if (
                    child.nodeType == child.TEXT_NODE
                    and child.nodeValue
                    and child.nodeValue.strip() == ""
                ) or child.nodeType == child.COMMENT_NODE:
                    element.removeChild(child)

        xml_file.write_bytes(dom.toxml(encoding="UTF-8"))
    except Exception as e:
        print(f"Warning: Failed to condense {xml_file.name}: {e}", file=sys.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Pack a directory into an Office file")
    parser.add_argument("input_directory", help="Unpacked Office document directory")
    parser.add_argument("output_file", help="Output file (.docx/.pptx/.xlsx)")
    args = parser.parse_args()

    message = pack(args.input_directory, args.output_file)
    print(message)
    if "Error" in message:
        sys.exit(1)
