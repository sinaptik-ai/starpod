# PDF Form Filling Guide

## Step 1: Check for fillable fields

```bash
python scripts/fill_form.py form.pdf --list
```

If fields are found, proceed to **Fillable Fields**. Otherwise, see **Non-Fillable Fields**.

## Fillable Fields

### Fill with pypdf
```python
from pypdf import PdfReader, PdfWriter

reader = PdfReader("form.pdf")
writer = PdfWriter()
writer.append(reader)

# Fill fields
writer.update_page_form_field_values(
    writer.pages[0],
    {"Name": "John Doe", "Date": "2025-01-15", "Email": "john@example.com"}
)

with open("filled.pdf", "wb") as f:
    writer.write(f)
```

### Fill checkboxes
```python
# Checkbox values are typically "/Yes" or "/Off"
writer.update_page_form_field_values(
    writer.pages[0],
    {"AgreeTerms": "/Yes", "OptOut": "/Off"}
)
```

### CLI
```bash
python scripts/fill_form.py form.pdf filled.pdf --fields '{"Name": "John", "Date": "2025-01-15"}'
python scripts/fill_form.py form.pdf filled.pdf --json field_values.json
```

## Non-Fillable Fields (Annotation-Based)

When a PDF has no fillable form fields, overlay text using reportlab annotations:

```python
from pypdf import PdfReader, PdfWriter
from reportlab.pdfgen import canvas
from reportlab.lib.pagesizes import letter
import io

# Create overlay with text at specific coordinates
packet = io.BytesIO()
c = canvas.Canvas(packet, pagesize=letter)
c.setFont("Helvetica", 12)

# Position text at exact coordinates (x, y from bottom-left)
c.drawString(150, 680, "John Doe")       # Name field
c.drawString(150, 650, "2025-01-15")     # Date field
c.drawString(150, 620, "john@example.com") # Email field
c.save()

# Merge overlay onto original
packet.seek(0)
overlay = PdfReader(packet)
original = PdfReader("form.pdf")
writer = PdfWriter()

for i, page in enumerate(original.pages):
    if i < len(overlay.pages):
        page.merge_page(overlay.pages[i])
    writer.add_page(page)

with open("filled.pdf", "wb") as f:
    writer.write(f)
```

### Finding field coordinates

Convert PDF to image first, then identify coordinates visually:
```bash
python scripts/convert_to_images.py form.pdf --dpi 150
```

PDF coordinates: origin is bottom-left, y increases upward. At 72 DPI:
- 1 inch = 72 points
- US Letter = 612 x 792 points
- A4 = 595 x 842 points
