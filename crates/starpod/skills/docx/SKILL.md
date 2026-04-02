---
name: docx
description: "Use this skill whenever the user wants to create, read, edit, or manipulate Word documents (.docx files). Triggers include: any mention of 'Word doc', '.docx', or requests to produce professional documents with headings, tables of contents, page numbers, or letterheads. Also use for extracting content from .docx files, working with tracked changes, or converting content into a polished Word document."
version: 0.1.0
compatibility: "Requires: pip install python-docx lxml. System: apt install libreoffice poppler-utils pandoc"
---

# DOCX Skill

## Quick Reference

| Task | Approach | Script |
|------|----------|--------|
| Read/extract text | python-docx or pandoc | See below |
| Create new document | python-docx (Python) or docx (Node.js) | See below |
| Edit existing | python-docx load → modify → save | See below |
| Edit XML directly | unpack → edit → pack | `python scripts/office/unpack.py doc.docx unpacked/` |
| Add comments | comment.py | `python scripts/comment.py unpacked/ 0 "text"` |
| Accept tracked changes | accept_changes.py | `python scripts/accept_changes.py in.docx out.docx` |
| Convert .doc → .docx | LibreOffice | `python scripts/office/soffice.py --headless --convert-to docx doc.doc` |
| Convert to PDF | LibreOffice | `python scripts/office/soffice.py --headless --convert-to pdf doc.docx` |

## Reading Content

### python-docx
```python
from docx import Document

doc = Document("document.docx")
for para in doc.paragraphs:
    print(f"[{para.style.name}] {para.text}")

for table in doc.tables:
    for row in table.rows:
        print([cell.text for cell in row.cells])
```

### pandoc (with tracked changes)
```bash
pandoc --track-changes=all document.docx -o output.md
```

## Creating New Documents with python-docx

### Full document example
```python
from docx import Document
from docx.shared import Inches, Pt, Cm, RGBColor
from docx.enum.text import WD_ALIGN_PARAGRAPH
from docx.enum.table import WD_TABLE_ALIGNMENT
from docx.enum.section import WD_ORIENT

doc = Document()

# Page setup (US Letter)
section = doc.sections[0]
section.page_width = Inches(8.5)
section.page_height = Inches(11)
section.top_margin = Inches(1)
section.bottom_margin = Inches(1)
section.left_margin = Inches(1)
section.right_margin = Inches(1)

# Default font
style = doc.styles['Normal']
font = style.font
font.name = 'Arial'
font.size = Pt(11)

# Title
title = doc.add_heading('Document Title', level=0)
title.alignment = WD_ALIGN_PARAGRAPH.CENTER

# Body text
doc.add_paragraph('Executive summary paragraph with key findings and recommendations.')

# Heading hierarchy
doc.add_heading('Section 1: Overview', level=1)
doc.add_paragraph('Section content goes here.')

doc.add_heading('1.1 Subsection', level=2)
doc.add_paragraph('Subsection details.')

doc.save('document.docx')
```

### Lists
```python
# Bullet list
doc.add_paragraph('First item', style='List Bullet')
doc.add_paragraph('Second item', style='List Bullet')

# Numbered list
doc.add_paragraph('Step one', style='List Number')
doc.add_paragraph('Step two', style='List Number')

# Nested bullets (indent level)
p = doc.add_paragraph('Sub-item', style='List Bullet 2')
```

### Tables
```python
table = doc.add_table(rows=4, cols=3)
table.style = 'Table Grid'
table.alignment = WD_TABLE_ALIGNMENT.CENTER

# Set column widths
for row in table.rows:
    row.cells[0].width = Inches(2)
    row.cells[1].width = Inches(3)
    row.cells[2].width = Inches(1.5)

# Header row
headers = ['Name', 'Description', 'Status']
for i, h in enumerate(headers):
    cell = table.rows[0].cells[i]
    cell.text = h
    for p in cell.paragraphs:
        p.runs[0].bold = True
    shading = cell._element.get_or_add_tcPr()
    from docx.oxml.ns import qn
    from lxml import etree
    shd = etree.SubElement(shading, qn('w:shd'))
    shd.set(qn('w:fill'), '333333')
    shd.set(qn('w:val'), 'clear')
    for p in cell.paragraphs:
        p.runs[0].font.color.rgb = RGBColor(0xFF, 0xFF, 0xFF)

# Data
data = [['Project A', 'Core platform rewrite', 'Active'],
        ['Project B', 'API integration', 'Planning'],
        ['Project C', 'Documentation update', 'Complete']]
for r, row_data in enumerate(data, 1):
    for c, val in enumerate(row_data):
        table.rows[r].cells[c].text = val
```

### Images
```python
doc.add_picture('chart.png', width=Inches(5))
last_paragraph = doc.paragraphs[-1]
last_paragraph.alignment = WD_ALIGN_PARAGRAPH.CENTER
```

### Headers and footers
```python
section = doc.sections[0]
header = section.header
header_para = header.paragraphs[0]
header_para.text = "Company Name"
header_para.style.font.size = Pt(9)

footer = section.footer
footer_para = footer.paragraphs[0]
footer_para.text = "Confidential"
footer_para.alignment = WD_ALIGN_PARAGRAPH.CENTER
```

### Page breaks
```python
doc.add_page_break()
```

### Landscape sections
```python
new_section = doc.add_section(WD_ORIENT.LANDSCAPE)
new_section.orientation = WD_ORIENT.LANDSCAPE
new_section.page_width = Inches(11)
new_section.page_height = Inches(8.5)
```

## Editing Existing Documents

```python
from docx import Document

doc = Document("existing.docx")

# Find and replace text
for para in doc.paragraphs:
    for run in para.runs:
        if "OLD_TEXT" in run.text:
            run.text = run.text.replace("OLD_TEXT", "NEW_TEXT")

# Add content at the end
doc.add_heading('New Section', level=1)
doc.add_paragraph('Additional content appended to the document.')

# Modify styles
for para in doc.paragraphs:
    if para.style.name == 'Heading 1':
        for run in para.runs:
            run.font.color.rgb = RGBColor(0x1E, 0x27, 0x61)

doc.save("modified.docx")
```

## Advanced: XML Editing Workflow

For complex modifications (TOC, footnotes, bookmarks, tracked changes) that python-docx can't handle, edit the raw XML:

```bash
# Unpack
python scripts/office/unpack.py document.docx unpacked/

# Edit XML files in unpacked/word/ using the Edit tool
# Key files: document.xml, styles.xml, [Content_Types].xml

# Add comments
python scripts/comment.py unpacked/ 0 "Review this section"

# Repack
python scripts/office/pack.py unpacked/ output.docx
```

### Key XML patterns

**Tracked change (replace "30" with "60"):**
```xml
<w:r><w:t>The term is </w:t></w:r>
<w:del w:id="1" w:author="Claude" w:date="2025-01-01T00:00:00Z">
  <w:r><w:delText>30</w:delText></w:r>
</w:del>
<w:ins w:id="2" w:author="Claude" w:date="2025-01-01T00:00:00Z">
  <w:r><w:t>60</w:t></w:r>
</w:ins>
<w:r><w:t> days.</w:t></w:r>
```

**Smart quotes in XML:**
```xml
<w:t>Here&#x2019;s a quote: &#x201C;Hello&#x201D;</w:t>
```

### DXA unit reference
- 1440 DXA = 1 inch
- US Letter: 12240 x 15840 DXA
- A4: 11906 x 16838 DXA

## Conversion

```bash
# .doc to .docx
python scripts/office/soffice.py --headless --convert-to docx document.doc

# .docx to PDF
python scripts/office/soffice.py --headless --convert-to pdf document.docx

# .docx to images
python scripts/office/soffice.py --headless --convert-to pdf document.docx
pdftoppm -jpeg -r 150 document.pdf page

# Accept all tracked changes (produces clean document)
python scripts/accept_changes.py input.docx clean.docx
```
