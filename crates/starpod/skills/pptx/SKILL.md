---
name: pptx
description: "Use this skill any time a .pptx file is involved — creating slide decks, reading or extracting content from presentations, editing existing slides, or converting between formats. Trigger whenever the user mentions 'deck', 'slides', 'presentation', or references a .pptx filename."
version: 0.1.0
compatibility: "Requires: pip install python-pptx markitdown[pptx] Pillow. System: apt install libreoffice poppler-utils"
---

# PPTX Skill

## Quick Reference

| Task | Approach | Script |
|------|----------|--------|
| Read/extract text | markitdown | `python -m markitdown presentation.pptx` |
| Visual overview | thumbnail grid | `python scripts/thumbnail.py deck.pptx` |
| Create from scratch | python-pptx | See below |
| Edit existing | python-pptx load → modify → save | See below |
| Edit XML directly | unpack → edit → pack | See `references/editing.md` |
| Clean orphans | remove unreferenced files | `python scripts/clean.py unpacked/` |
| Convert to images | LibreOffice → pdftoppm | See below |

## Reading Content

```bash
python -m markitdown presentation.pptx
python scripts/thumbnail.py presentation.pptx    # visual grid
```

## Creating Presentations with python-pptx

### Basic deck
```python
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN

prs = Presentation()
prs.slide_width = Inches(13.333)   # 16:9 widescreen
prs.slide_height = Inches(7.5)

# Title slide
slide = prs.slides.add_slide(prs.slide_layouts[6])  # blank layout
txBox = slide.shapes.add_textbox(Inches(1), Inches(2.5), Inches(11), Inches(2))
tf = txBox.text_frame
p = tf.paragraphs[0]
p.text = "Presentation Title"
p.font.size = Pt(44)
p.font.bold = True
p.font.color.rgb = RGBColor(0x1E, 0x27, 0x61)
p.alignment = PP_ALIGN.CENTER

prs.save("output.pptx")
```

### Slide with image and text columns
```python
slide = prs.slides.add_slide(prs.slide_layouts[6])

# Left column: text
txBox = slide.shapes.add_textbox(Inches(0.8), Inches(1.2), Inches(5.5), Inches(5))
tf = txBox.text_frame
tf.word_wrap = True
p = tf.paragraphs[0]
p.text = "Key Insight"
p.font.size = Pt(28)
p.font.bold = True

p2 = tf.add_paragraph()
p2.text = "Supporting details go here with context and evidence."
p2.font.size = Pt(16)
p2.space_before = Pt(12)

# Right column: image
slide.shapes.add_picture("chart.png", Inches(7), Inches(1), Inches(5.5), Inches(5))
```

### Tables
```python
rows, cols = 4, 3
table_shape = slide.shapes.add_table(rows, cols, Inches(1), Inches(1.5), Inches(11), Inches(4))
table = table_shape.table

# Headers
headers = ["Metric", "Q1", "Q2"]
for i, h in enumerate(headers):
    cell = table.cell(0, i)
    cell.text = h
    for p in cell.text_frame.paragraphs:
        p.font.bold = True
        p.font.size = Pt(14)
        p.font.color.rgb = RGBColor(0xFF, 0xFF, 0xFF)
    cell.fill.solid()
    cell.fill.fore_color.rgb = RGBColor(0x1E, 0x27, 0x61)

# Data rows
data = [["Revenue", "$1.2M", "$1.5M"], ["Growth", "12%", "15%"], ["Users", "5,000", "6,200"]]
for r, row_data in enumerate(data):
    for c, val in enumerate(row_data):
        table.cell(r + 1, c).text = val
```

### Charts
```python
from pptx.chart.data import CategoryChartData
from pptx.enum.chart import XL_CHART_TYPE

chart_data = CategoryChartData()
chart_data.categories = ['Q1', 'Q2', 'Q3', 'Q4']
chart_data.add_series('Revenue', (1.2, 1.5, 1.8, 2.1))
chart_data.add_series('Profit', (0.3, 0.4, 0.5, 0.7))

chart_frame = slide.shapes.add_chart(
    XL_CHART_TYPE.COLUMN_CLUSTERED,
    Inches(1), Inches(1.5), Inches(11), Inches(5),
    chart_data
)
```

### Shapes and backgrounds
```python
from pptx.enum.shapes import MSO_SHAPE

# Colored rectangle background
shape = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0), Inches(0), Inches(13.333), Inches(7.5))
shape.fill.solid()
shape.fill.fore_color.rgb = RGBColor(0x1E, 0x27, 0x61)
shape.line.fill.background()  # no border

# Accent shape
accent = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.8), Inches(6.5), Inches(2), Inches(0.08))
accent.fill.solid()
accent.fill.fore_color.rgb = RGBColor(0xF9, 0x61, 0x67)
accent.line.fill.background()
```

## Design Guidelines

### Color Palettes — pick one that matches the topic

| Theme | Primary | Secondary | Accent |
|-------|---------|-----------|--------|
| **Midnight Executive** | `1E2761` | `CADCFC` | `FFFFFF` |
| **Forest & Moss** | `2C5F2D` | `97BC62` | `F5F5F5` |
| **Coral Energy** | `F96167` | `F9E795` | `2F3C7E` |
| **Warm Terracotta** | `B85042` | `E7E8D1` | `A7BEAE` |
| **Ocean Gradient** | `065A82` | `1C7293` | `21295C` |
| **Charcoal Minimal** | `36454F` | `F2F2F2` | `212121` |

### Typography

| Element | Size |
|---------|------|
| Slide title | 36–44pt bold |
| Section header | 20–24pt bold |
| Body text | 14–16pt |
| Captions | 10–12pt muted |

### Layout Principles

- **Every slide needs a visual element** — image, chart, icon, or shape. No text-only slides.
- **Vary layouts** — alternate between two-column, full-bleed image, grid, and callout layouts.
- **Large stat callouts** — big numbers at 60–72pt with small labels below.
- **0.5" minimum margins**, 0.3–0.5" between content blocks.
- **Left-align body text** — center only titles.
- **Dark backgrounds for title/conclusion**, light for content slides.

### Avoid

- Repeating the same layout across slides
- Default blue — pick colors that match the topic
- Text-only slides without visual elements
- Low-contrast text or icons
- Accent lines under titles (looks AI-generated)

## Editing Existing Presentations

```python
from pptx import Presentation

prs = Presentation("existing.pptx")
for slide in prs.slides:
    for shape in slide.shapes:
        if shape.has_text_frame:
            for paragraph in shape.text_frame.paragraphs:
                for run in paragraph.runs:
                    if "OLD TEXT" in run.text:
                        run.text = run.text.replace("OLD TEXT", "NEW TEXT")
prs.save("modified.pptx")
```

## Converting to Images

```bash
libreoffice --headless --convert-to pdf output.pptx
pdftoppm -jpeg -r 150 output.pdf slide
# Creates slide-01.jpg, slide-02.jpg, etc.
```

## QA Checklist

1. Extract text: `python -m markitdown output.pptx` — check for missing/wrong content
2. Convert to images and visually inspect every slide
3. Check for: overlapping elements, text overflow, low contrast, uneven spacing, leftover placeholders
4. Fix issues, re-render, re-inspect — first render is rarely perfect
