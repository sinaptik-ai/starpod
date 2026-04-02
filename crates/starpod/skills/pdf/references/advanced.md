# PDF Advanced Reference

## pypdfium2 (Chromium's PDFium)

Fast rendering and text extraction. Apache/BSD licensed.

### Render PDF to images
```python
import pypdfium2 as pdfium

pdf = pdfium.PdfDocument("document.pdf")
for i, page in enumerate(pdf):
    bitmap = page.render(scale=2.0)
    img = bitmap.to_pil()
    img.save(f"page_{i+1}.png", "PNG")
```

### Extract text
```python
import pypdfium2 as pdfium

pdf = pdfium.PdfDocument("document.pdf")
for i, page in enumerate(pdf):
    text = page.get_text()
    print(f"Page {i+1}: {len(text)} chars")
```

## pdf-lib (JavaScript)

Create and modify PDFs in Node.js or browser.

### Create PDF
```javascript
const { PDFDocument, rgb, StandardFonts } = require('pdf-lib');
const fs = require('fs');

async function createPdf() {
    const doc = await PDFDocument.create();
    const page = doc.addPage([612, 792]); // US Letter
    const font = await doc.embedFont(StandardFonts.Helvetica);

    page.drawText('Hello World', { x: 50, y: 700, size: 24, font, color: rgb(0, 0, 0) });

    const bytes = await doc.save();
    fs.writeFileSync('output.pdf', bytes);
}
createPdf();
```

### Modify existing PDF
```javascript
const { PDFDocument } = require('pdf-lib');
const fs = require('fs');

async function modifyPdf() {
    const bytes = fs.readFileSync('input.pdf');
    const doc = await PDFDocument.load(bytes);
    const pages = doc.getPages();

    pages[0].drawText('CONFIDENTIAL', { x: 200, y: 400, size: 48, opacity: 0.3 });

    const modified = await doc.save();
    fs.writeFileSync('modified.pdf', modified);
}
modifyPdf();
```

### Fill forms with pdf-lib
```javascript
const { PDFDocument } = require('pdf-lib');
const fs = require('fs');

async function fillForm() {
    const bytes = fs.readFileSync('form.pdf');
    const doc = await PDFDocument.load(bytes);
    const form = doc.getForm();

    form.getTextField('Name').setText('John Doe');
    form.getTextField('Date').setText('2025-01-15');
    form.getCheckBox('Agree').check();

    const filled = await doc.save();
    fs.writeFileSync('filled.pdf', filled);
}
fillForm();
```

## Batch Processing

### Process directory of PDFs
```python
from pathlib import Path
from pypdf import PdfReader

for pdf_file in sorted(Path(".").glob("*.pdf")):
    reader = PdfReader(pdf_file)
    text = "".join(page.extract_text() or "" for page in reader.pages)
    print(f"{pdf_file.name}: {len(reader.pages)} pages, {len(text)} chars")
```

### Combine specific page ranges
```python
from pypdf import PdfReader, PdfWriter

specs = [
    ("report.pdf", range(0, 5)),    # Pages 1-5
    ("appendix.pdf", range(2, 10)), # Pages 3-10
    ("cover.pdf", range(0, 1)),     # Page 1 only
]

writer = PdfWriter()
for filename, pages in specs:
    reader = PdfReader(filename)
    for i in pages:
        if i < len(reader.pages):
            writer.add_page(reader.pages[i])

with open("combined.pdf", "wb") as f:
    writer.write(f)
```
