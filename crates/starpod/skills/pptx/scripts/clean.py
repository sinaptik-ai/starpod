"""Remove unreferenced files from an unpacked PPTX directory.

Cleans orphaned slides, media, charts, diagrams, and stale content types.

Usage:
    python clean.py unpacked/
"""

import sys
from pathlib import Path
from xml.dom import minidom
import re


def get_slides_in_sldidlst(unpacked_dir: Path) -> set[str]:
    pres_path = unpacked_dir / "ppt" / "presentation.xml"
    pres_rels = unpacked_dir / "ppt" / "_rels" / "presentation.xml.rels"

    if not pres_path.exists() or not pres_rels.exists():
        return set()

    rels_dom = minidom.parse(str(pres_rels))
    rid_to_slide = {}
    for rel in rels_dom.getElementsByTagName("Relationship"):
        target = rel.getAttribute("Target")
        if "slide" in rel.getAttribute("Type") and target.startswith("slides/"):
            rid_to_slide[rel.getAttribute("Id")] = target.replace("slides/", "")

    content = pres_path.read_text(encoding="utf-8")
    rids = set(re.findall(r'<p:sldId[^>]*r:id="([^"]+)"', content))
    return {rid_to_slide[r] for r in rids if r in rid_to_slide}


def get_referenced_files(unpacked_dir: Path) -> set:
    referenced = set()
    for rels_file in unpacked_dir.rglob("*.rels"):
        dom = minidom.parse(str(rels_file))
        for rel in dom.getElementsByTagName("Relationship"):
            target = rel.getAttribute("Target")
            if not target:
                continue
            target_path = (rels_file.parent.parent / target).resolve()
            try:
                referenced.add(target_path.relative_to(unpacked_dir.resolve()))
            except ValueError:
                pass
    return referenced


def clean_unused_files(unpacked_dir: Path) -> list[str]:
    all_removed = []
    referenced_slides = get_slides_in_sldidlst(unpacked_dir)

    # Remove orphaned slides
    slides_dir = unpacked_dir / "ppt" / "slides"
    if slides_dir.exists():
        for slide_file in slides_dir.glob("slide*.xml"):
            if slide_file.name not in referenced_slides:
                rel_path = str(slide_file.relative_to(unpacked_dir))
                slide_file.unlink()
                all_removed.append(rel_path)
                rels_file = slides_dir / "_rels" / f"{slide_file.name}.rels"
                if rels_file.exists():
                    rels_file.unlink()
                    all_removed.append(str(rels_file.relative_to(unpacked_dir)))

    # Remove [trash] directory
    trash_dir = unpacked_dir / "[trash]"
    if trash_dir.exists() and trash_dir.is_dir():
        for f in trash_dir.iterdir():
            if f.is_file():
                all_removed.append(str(f.relative_to(unpacked_dir)))
                f.unlink()
        trash_dir.rmdir()

    # Iteratively remove unreferenced media, charts, diagrams, etc.
    while True:
        referenced = get_referenced_files(unpacked_dir)
        removed_this_pass = []

        for dir_name in ["media", "embeddings", "charts", "diagrams", "drawings", "ink"]:
            dir_path = unpacked_dir / "ppt" / dir_name
            if not dir_path.exists():
                continue
            for f in dir_path.glob("*"):
                if f.is_file():
                    rel = f.relative_to(unpacked_dir)
                    if rel not in referenced:
                        f.unlink()
                        removed_this_pass.append(str(rel))

        if not removed_this_pass:
            break
        all_removed.extend(removed_this_pass)

    # Update [Content_Types].xml
    if all_removed:
        ct_path = unpacked_dir / "[Content_Types].xml"
        if ct_path.exists():
            dom = minidom.parse(str(ct_path))
            changed = False
            for override in list(dom.getElementsByTagName("Override")):
                part = override.getAttribute("PartName").lstrip("/")
                if part in all_removed:
                    override.parentNode.removeChild(override)
                    changed = True
            if changed:
                ct_path.write_bytes(dom.toxml(encoding="utf-8"))

    return all_removed


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python clean.py <unpacked_dir>", file=sys.stderr)
        sys.exit(1)

    d = Path(sys.argv[1])
    if not d.exists():
        print(f"Error: {d} not found", file=sys.stderr)
        sys.exit(1)

    removed = clean_unused_files(d)
    if removed:
        print(f"Removed {len(removed)} unreferenced files:")
        for f in removed:
            print(f"  {f}")
    else:
        print("No unreferenced files found")
