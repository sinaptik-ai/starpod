"""Add comments to unpacked DOCX documents.

Handles all the boilerplate: comments.xml, commentsExtended.xml,
commentsIds.xml, relationships, and content types.

Usage:
    python comment.py unpacked/ 0 "Comment text"
    python comment.py unpacked/ 1 "Reply text" --parent 0
    python comment.py unpacked/ 0 "Text" --author "Custom Author"

Text should be pre-escaped XML (e.g., &amp; for &, &#x2019; for smart quotes).

After running, add markers to document.xml:
  <w:commentRangeStart w:id="0"/>
  ... commented content ...
  <w:commentRangeEnd w:id="0"/>
  <w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r>
"""

import argparse
import random
import sys
from datetime import datetime, timezone
from pathlib import Path
from xml.dom import minidom

NS = {
    "w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "w14": "http://schemas.microsoft.com/office/word/2010/wordml",
    "w15": "http://schemas.microsoft.com/office/word/2012/wordml",
    "w16cid": "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    "w16cex": "http://schemas.microsoft.com/office/word/2018/wordml/cex",
}

COMMENTS_TEMPLATE = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"></w:comments>'
COMMENTS_EXT_TEMPLATE = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"></w15:commentsEx>'
COMMENTS_IDS_TEMPLATE = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"></w16cid:commentsIds>'
COMMENTS_EXTENSIBLE_TEMPLATE = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w16cex:commentsExtensible xmlns:w16cex="http://schemas.microsoft.com/office/word/2018/wordml/cex"></w16cex:commentsExtensible>'

COMMENT_XML = """\
<w:comment w:id="{id}" w:author="{author}" w:date="{date}" w:initials="{initials}">
  <w:p w14:paraId="{para_id}" w14:textId="77777777">
    <w:r>
      <w:rPr><w:rStyle w:val="CommentReference"/></w:rPr>
      <w:annotationRef/>
    </w:r>
    <w:r>
      <w:rPr><w:color w:val="000000"/><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr>
      <w:t>{text}</w:t>
    </w:r>
  </w:p>
</w:comment>"""


def _hex_id() -> str:
    return f"{random.randint(0, 0x7FFFFFFE):08X}"


def _append_xml(xml_path: Path, root_tag: str, content: str) -> None:
    dom = minidom.parseString(xml_path.read_text(encoding="utf-8"))
    root = dom.getElementsByTagName(root_tag)[0]
    ns_attrs = " ".join(f'xmlns:{k}="{v}"' for k, v in NS.items())
    wrapper = minidom.parseString(f"<root {ns_attrs}>{content}</root>")
    for child in wrapper.documentElement.childNodes:
        if child.nodeType == child.ELEMENT_NODE:
            root.appendChild(dom.importNode(child, True))
    xml_path.write_bytes(dom.toxml(encoding="UTF-8"))


def _find_para_id(comments_path: Path, comment_id: int) -> str | None:
    dom = minidom.parseString(comments_path.read_text(encoding="utf-8"))
    for c in dom.getElementsByTagName("w:comment"):
        if c.getAttribute("w:id") == str(comment_id):
            for p in c.getElementsByTagName("w:p"):
                pid = p.getAttribute("w14:paraId")
                if pid:
                    return pid
    return None


def _ensure_relationships(unpacked_dir: Path) -> None:
    rels_path = unpacked_dir / "word" / "_rels" / "document.xml.rels"
    if not rels_path.exists():
        return

    dom = minidom.parseString(rels_path.read_text(encoding="utf-8"))
    existing = {r.getAttribute("Target") for r in dom.getElementsByTagName("Relationship")}
    if "comments.xml" in existing:
        return

    root = dom.documentElement
    next_rid = max(
        (int(r.getAttribute("Id")[3:]) for r in dom.getElementsByTagName("Relationship")
         if r.getAttribute("Id").startswith("rId")),
        default=0,
    ) + 1

    rels = [
        ("http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments", "comments.xml"),
        ("http://schemas.microsoft.com/office/2011/relationships/commentsExtended", "commentsExtended.xml"),
        ("http://schemas.microsoft.com/office/2016/09/relationships/commentsIds", "commentsIds.xml"),
        ("http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible", "commentsExtensible.xml"),
    ]
    for rel_type, target in rels:
        rel = dom.createElement("Relationship")
        rel.setAttribute("Id", f"rId{next_rid}")
        rel.setAttribute("Type", rel_type)
        rel.setAttribute("Target", target)
        root.appendChild(rel)
        next_rid += 1

    rels_path.write_bytes(dom.toxml(encoding="UTF-8"))


def _ensure_content_types(unpacked_dir: Path) -> None:
    ct_path = unpacked_dir / "[Content_Types].xml"
    if not ct_path.exists():
        return

    dom = minidom.parseString(ct_path.read_text(encoding="utf-8"))
    existing = {o.getAttribute("PartName") for o in dom.getElementsByTagName("Override")}
    if "/word/comments.xml" in existing:
        return

    root = dom.documentElement
    overrides = [
        ("/word/comments.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
        ("/word/commentsExtended.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"),
        ("/word/commentsIds.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsIds+xml"),
        ("/word/commentsExtensible.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtensible+xml"),
    ]
    for part_name, content_type in overrides:
        o = dom.createElement("Override")
        o.setAttribute("PartName", part_name)
        o.setAttribute("ContentType", content_type)
        root.appendChild(o)

    ct_path.write_bytes(dom.toxml(encoding="UTF-8"))


def add_comment(
    unpacked_dir: str,
    comment_id: int,
    text: str,
    author: str = "Claude",
    initials: str = "C",
    parent_id: int | None = None,
) -> tuple[str, str]:
    word = Path(unpacked_dir) / "word"
    if not word.exists():
        return "", f"Error: {word} not found"

    para_id = _hex_id()
    durable_id = _hex_id()
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    # Ensure comments.xml exists
    comments = word / "comments.xml"
    if not comments.exists():
        comments.write_text(COMMENTS_TEMPLATE, encoding="utf-8")
        _ensure_relationships(Path(unpacked_dir))
        _ensure_content_types(Path(unpacked_dir))

    _append_xml(comments, "w:comments", COMMENT_XML.format(
        id=comment_id, author=author, date=ts, initials=initials,
        para_id=para_id, text=text,
    ))

    # commentsExtended.xml
    ext = word / "commentsExtended.xml"
    if not ext.exists():
        ext.write_text(COMMENTS_EXT_TEMPLATE, encoding="utf-8")
    if parent_id is not None:
        parent_para = _find_para_id(comments, parent_id)
        if not parent_para:
            return "", f"Error: Parent comment {parent_id} not found"
        _append_xml(ext, "w15:commentsEx",
                    f'<w15:commentEx w15:paraId="{para_id}" w15:paraIdParent="{parent_para}" w15:done="0"/>')
    else:
        _append_xml(ext, "w15:commentsEx",
                    f'<w15:commentEx w15:paraId="{para_id}" w15:done="0"/>')

    # commentsIds.xml
    ids = word / "commentsIds.xml"
    if not ids.exists():
        ids.write_text(COMMENTS_IDS_TEMPLATE, encoding="utf-8")
    _append_xml(ids, "w16cid:commentsIds",
                f'<w16cid:commentId w16cid:paraId="{para_id}" w16cid:durableId="{durable_id}"/>')

    # commentsExtensible.xml
    extensible = word / "commentsExtensible.xml"
    if not extensible.exists():
        extensible.write_text(COMMENTS_EXTENSIBLE_TEMPLATE, encoding="utf-8")
    _append_xml(extensible, "w16cex:commentsExtensible",
                f'<w16cex:commentExtensible w16cex:durableId="{durable_id}" w16cex:dateUtc="{ts}"/>')

    action = "reply" if parent_id is not None else "comment"
    return para_id, f"Added {action} {comment_id} (para_id={para_id})"


if __name__ == "__main__":
    p = argparse.ArgumentParser(description="Add comments to DOCX documents")
    p.add_argument("unpacked_dir", help="Unpacked DOCX directory")
    p.add_argument("comment_id", type=int, help="Comment ID (must be unique)")
    p.add_argument("text", help="Comment text (pre-escaped XML)")
    p.add_argument("--author", default="Claude", help="Author name")
    p.add_argument("--initials", default="C", help="Author initials")
    p.add_argument("--parent", type=int, help="Parent comment ID (for replies)")
    args = p.parse_args()

    para_id, msg = add_comment(args.unpacked_dir, args.comment_id, args.text,
                                args.author, args.initials, args.parent)
    print(msg)
    if "Error" in msg:
        sys.exit(1)

    cid = args.comment_id
    if args.parent is not None:
        print(f"\nNest markers inside parent {args.parent}'s markers:")
        print(f'  <w:commentRangeStart w:id="{args.parent}"/><w:commentRangeStart w:id="{cid}"/>')
        print(f'  <w:r>...</w:r>')
        print(f'  <w:commentRangeEnd w:id="{cid}"/><w:commentRangeEnd w:id="{args.parent}"/>')
        print(f'  <w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="{args.parent}"/></w:r>')
        print(f'  <w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="{cid}"/></w:r>')
    else:
        print(f"\nAdd to document.xml:")
        print(f'  <w:commentRangeStart w:id="{cid}"/>')
        print(f'  <w:r>...</w:r>')
        print(f'  <w:commentRangeEnd w:id="{cid}"/>')
        print(f'  <w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="{cid}"/></w:r>')
