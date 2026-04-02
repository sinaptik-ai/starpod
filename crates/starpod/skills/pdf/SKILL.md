---
name: pdf
description: "Use this skill whenever the user wants to do anything with PDF files — reading, extracting text or tables, merging, splitting, rotating, creating new PDFs, adding watermarks, OCR on scanned documents, encrypting/decrypting, or filling forms. Trigger whenever a .pdf file is mentioned or needs to be produced."
version: 0.1.0
compatibility: "Requires: pip install pypdf pdfplumber reportlab. System: apt install poppler-utils. OCR: apt install tesseract-ocr; pip install pytesseract pdf2image"
---

# PDF Processing

## Quick Reference

| Task | Tool | Script |
|------|------|--------|
| Extract text | pdfplumber | `page.extract_text()` |
| Extract tables | pdfplumber | `python scripts/extract_tables.py input.pdf` |
| Merge PDFs | pypdf | `python scripts/merge.py output.pdf a.pdf b.pdf` |
| Convert to images | poppler-utils | `python scripts/convert_to_images.py input.pdf` |
| Fill form fields | pypdf | `python scripts/fill_form.py form.pdf --list` |
| Create new PDF | reportlab | `SimpleDocTemplate` or `Canvas` |
| OCR scanned PDF | pytesseract + pdf2image | `image_to_string()` |
| CLI text extraction | poppler-utils | `pdftotext` |
| CLI merge/split | qpdf | `qpdf --empty --pages` |

For form filling workflow, see `references/forms.md`.
For advanced libraries (pypdfium2, pdf-lib), see `references/advanced.md`.

## Reading & Extracting

### Text extraction
```python
import pdfplumber

with pdfplumber.open("document.pdf") as pdf:
    for page in pdf.pages:
        print(page.extract_text())
```

### Table extraction → DataFrame
```python
import pdfplumber, pandas as pd

with pdfplumber.open("document.pdf") as pdf:
    tables = []
    for page in pdf.pages:
        for table in page.extract_tables():
            if table:
                df = pd.DataFrame(table[1:], columns=table[0])
                tables.append(df)
    if tables:
        combined = pd.concat(tables, ignore_index=True)
        combined.to_excel("tables.xlsx", index=False)
```

### Metadata
```python
from pypdf import PdfReader
reader = PdfReader("document.pdf")
meta = reader.metadata
print(f"Title: {meta.title}, Author: {meta.author}, Pages: {len(reader.pages)}")
```

## Merging & Splitting

### Merge multiple PDFs
```python
from pypdf import PdfWriter, PdfReader

writer = PdfWriter()
for path in ["a.pdf", "b.pdf", "c.pdf"]:
    for page in PdfReader(path).pages:
        writer.add_page(page)
with open("merged.pdf", "wb") as f:
    writer.write(f)
```

### Split into individual pages
```python
from pypdf import PdfReader, PdfWriter

for i, page in enumerate(PdfReader("input.pdf").pages):
    w = PdfWriter()
    w.add_page(page)
    with open(f"page_{i+1}.pdf", "wb") as f:
        w.write(f)
```

### Extract page range
```python
from pypdf import PdfReader, PdfWriter

reader = PdfReader("input.pdf")
writer = PdfWriter()
for page in reader.pages[2:7]:  # pages 3-7 (0-indexed)
    writer.add_page(page)
with open("excerpt.pdf", "wb") as f:
    writer.write(f)
```

## Rotating Pages

```python
from pypdf import PdfReader, PdfWriter

reader = PdfReader("input.pdf")
writer = PdfWriter()
for page in reader.pages:
    page.rotate(90)  # 90, 180, 270
    writer.add_page(page)
with open("rotated.pdf", "wb") as f:
    writer.write(f)
```

## Creating New PDFs

### Simple document with reportlab
```python
from reportlab.lib.pagesizes import letter
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle, PageBreak
from reportlab.lib.styles import getSampleStyleSheet
from reportlab.lib import colors

doc = SimpleDocTemplate("report.pdf", pagesize=letter)
styles = getSampleStyleSheet()
story = []

story.append(Paragraph("Report Title", styles['Title']))
story.append(Spacer(1, 24))
story.append(Paragraph("Body text goes here. " * 10, styles['Normal']))

# Table
data = [["Name", "Score"], ["Alice", "95"], ["Bob", "87"]]
t = Table(data, colWidths=[200, 100])
t.setStyle(TableStyle([
    ('BACKGROUND', (0, 0), (-1, 0), colors.HexColor('#333333')),
    ('TEXTCOLOR', (0, 0), (-1, 0), colors.white),
    ('GRID', (0, 0), (-1, -1), 0.5, colors.grey),
    ('FONTSIZE', (0, 0), (-1, -1), 10),
]))
story.append(t)
story.append(PageBreak())
story.append(Paragraph("Page 2 content", styles['Heading1']))

doc.build(story)
```

### CRITICAL: Subscripts and superscripts in reportlab

**Never use Unicode subscript/superscript characters** (₀₁₂₃₄₅₆₇₈₉, ⁰¹²³⁴⁵⁶⁷⁸⁹) — built-in fonts render them as black boxes.

Use XML markup in Paragraph objects instead:
```python
Paragraph("H<sub>2</sub>O and x<super>2</super>", styles['Normal'])
```

## Watermarks & Security

### Add watermark
```python
from pypdf import PdfReader, PdfWriter

watermark = PdfReader("watermark.pdf").pages[0]
reader = PdfReader("document.pdf")
writer = PdfWriter()
for page in reader.pages:
    page.merge_page(watermark)
    writer.add_page(page)
with open("watermarked.pdf", "wb") as f:
    writer.write(f)
```

### Password protection
```python
from pypdf import PdfReader, PdfWriter

reader = PdfReader("input.pdf")
writer = PdfWriter()
for page in reader.pages:
    writer.add_page(page)
writer.encrypt("userpass", "ownerpass")
with open("encrypted.pdf", "wb") as f:
    writer.write(f)
```

## OCR for Scanned PDFs

```python
import pytesseract
from pdf2image import convert_from_path

images = convert_from_path("scanned.pdf")
text = "\n\n".join(
    f"--- Page {i+1} ---\n{pytesseract.image_to_string(img)}"
    for i, img in enumerate(images)
)
```

Requires: `pip install pytesseract pdf2image` and system packages `tesseract-ocr`, `poppler-utils`.

## CLI Quick Reference

```bash
# Text extraction (poppler-utils)
pdftotext input.pdf output.txt
pdftotext -layout input.pdf output.txt       # preserve layout
pdftotext -f 1 -l 5 input.pdf output.txt     # pages 1-5

# Merge (qpdf)
qpdf --empty --pages a.pdf b.pdf -- merged.pdf

# Split page range
qpdf input.pdf --pages . 1-5 -- first5.pdf

# Rotate page 1 by 90°
qpdf input.pdf output.pdf --rotate=+90:1

# Decrypt
qpdf --password=PASS --decrypt encrypted.pdf decrypted.pdf

# Extract images (poppler-utils)
pdfimages -j input.pdf prefix    # outputs prefix-000.jpg, etc.
```
