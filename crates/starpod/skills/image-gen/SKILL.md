---
name: image-gen
description: "Use this skill when the user asks to create, edit, or manipulate images programmatically — including diagrams, charts, infographics, thumbnails, social media graphics, image processing (resize, crop, filter, watermark), or generating visual assets from data. Trigger when the output is a .png, .jpg, .svg, or .gif file."
version: 0.1.0
compatibility: "Requires: pip install Pillow matplotlib seaborn. System: apt install fonts-dejavu-core"
---

# Image Generation & Processing

## Quick Reference

| Task | Tool | Script |
|------|------|--------|
| Batch resize | Pillow | `python scripts/resize.py img.jpg --width 800` |
| Add watermark | Pillow | `python scripts/watermark.py img.jpg "DRAFT"` |
| Data charts | matplotlib + seaborn | See below |
| Diagrams / flowcharts | matplotlib + patches, or SVG | See below |
| Image processing | Pillow (PIL) | See below |
| Social media graphics | Pillow with text + shapes | See below |
| Simple icons / logos | SVG (write XML directly) | See below |
| Animated GIFs | Pillow frame assembly | See below |

## Pillow — Image Processing

### Resize and crop
```python
from PIL import Image

img = Image.open("input.jpg")

# Resize maintaining aspect ratio
img.thumbnail((800, 600))
img.save("resized.jpg")

# Exact resize (may distort)
img_resized = img.resize((800, 600))

# Crop (left, upper, right, lower)
cropped = img.crop((100, 50, 500, 400))
cropped.save("cropped.jpg")

# Center crop to square
w, h = img.size
size = min(w, h)
left = (w - size) // 2
top = (h - size) // 2
square = img.crop((left, top, left + size, top + size))
```

### Filters and adjustments
```python
from PIL import ImageFilter, ImageEnhance

# Blur / sharpen
blurred = img.filter(ImageFilter.GaussianBlur(radius=3))
sharpened = img.filter(ImageFilter.SHARPEN)

# Brightness, contrast, saturation
enhancer = ImageEnhance.Brightness(img)
bright = enhancer.enhance(1.3)  # 1.0 = original

enhancer = ImageEnhance.Contrast(img)
contrast = enhancer.enhance(1.5)

# Convert to grayscale
gray = img.convert("L")
```

### Compositing and watermarks
```python
from PIL import Image, ImageDraw, ImageFont

img = Image.open("photo.jpg")
draw = ImageDraw.Draw(img)

# Text watermark
font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 36)
draw.text((20, img.height - 50), "© 2025", fill=(255, 255, 255, 128), font=font)

# Overlay image (logo in corner)
logo = Image.open("logo.png").resize((100, 100))
img.paste(logo, (img.width - 110, 10), logo)  # third arg = mask for transparency

img.save("watermarked.jpg")
```

## Pillow — Creating Graphics from Scratch

### Banner / social media graphic
```python
from PIL import Image, ImageDraw, ImageFont

W, H = 1200, 630  # Open Graph size
img = Image.new("RGB", (W, H), "#1E2761")
draw = ImageDraw.Draw(img)

# Background accent shape
draw.rectangle([(0, H - 80), (W, H)], fill="#F96167")

# Title text
font_title = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 64)
font_sub = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 28)
# Install fonts if missing: apt install fonts-dejavu-core

# Center title
bbox = draw.textbbox((0, 0), "Project Launch", font=font_title)
tw = bbox[2] - bbox[0]
draw.text(((W - tw) // 2, 200), "Project Launch", fill="white", font=font_title)

# Subtitle
bbox = draw.textbbox((0, 0), "Coming Q2 2025", font=font_sub)
tw = bbox[2] - bbox[0]
draw.text(((W - tw) // 2, 300), "Coming Q2 2025", fill="#CADCFC", font=font_sub)

img.save("banner.png")
```

### Grid / contact sheet
```python
from PIL import Image
import glob

images = [Image.open(f) for f in sorted(glob.glob("photos/*.jpg"))[:9]]
thumb_size = (300, 300)
cols, rows = 3, 3
padding = 10
bg_color = "#0A0A0A"

canvas_w = cols * thumb_size[0] + (cols + 1) * padding
canvas_h = rows * thumb_size[1] + (rows + 1) * padding
canvas = Image.new("RGB", (canvas_w, canvas_h), bg_color)

for i, img in enumerate(images):
    img.thumbnail(thumb_size)
    r, c = divmod(i, cols)
    x = padding + c * (thumb_size[0] + padding)
    y = padding + r * (thumb_size[1] + padding)
    canvas.paste(img, (x, y))

canvas.save("grid.png")
```

## Diagrams with matplotlib

### Flowchart
```python
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches

fig, ax = plt.subplots(figsize=(10, 8))
ax.set_xlim(0, 10)
ax.set_ylim(0, 10)
ax.axis('off')

def add_box(ax, x, y, text, color='#1E2761', w=2.5, h=0.8):
    rect = mpatches.FancyBboxPatch((x - w/2, y - h/2), w, h,
        boxstyle="round,pad=0.1", facecolor=color, edgecolor='white', linewidth=1.5)
    ax.add_patch(rect)
    ax.text(x, y, text, ha='center', va='center', fontsize=11,
            color='white', fontweight='bold')

def add_arrow(ax, x1, y1, x2, y2):
    ax.annotate('', xy=(x2, y2), xytext=(x1, y1),
        arrowprops=dict(arrowstyle='->', color='#888888', lw=1.5))

add_box(ax, 5, 9, "Start")
add_arrow(ax, 5, 8.6, 5, 7.8)
add_box(ax, 5, 7.4, "Process Data")
add_arrow(ax, 5, 7.0, 5, 6.2)
add_box(ax, 5, 5.8, "Analyze Results")
add_arrow(ax, 5, 5.4, 5, 4.6)
add_box(ax, 5, 4.2, "Generate Report", color='#2C5F2D')

plt.tight_layout()
plt.savefig('flowchart.png', dpi=150, bbox_inches='tight', facecolor='#0A0A0A')
```

## SVG — Simple Vector Graphics

```python
svg = '''<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
  <rect width="400" height="300" fill="#0A0A0A"/>
  <circle cx="200" cy="150" r="80" fill="none" stroke="#C0C0C0" stroke-width="2"/>
  <text x="200" y="155" text-anchor="middle" fill="#E8E8E8"
        font-family="system-ui" font-size="18" font-weight="bold">
    Logo Text
  </text>
</svg>'''

with open("icon.svg", "w") as f:
    f.write(svg)
```

## Animated GIF

```python
from PIL import Image, ImageDraw

frames = []
W, H = 400, 400

for i in range(30):
    img = Image.new("RGB", (W, H), "#0A0A0A")
    draw = ImageDraw.Draw(img)
    x = int(i / 29 * (W - 60))
    draw.ellipse([x, 170, x + 60, 230], fill="#F96167")
    frames.append(img)

frames[0].save("animation.gif", save_all=True, append_images=frames[1:],
               duration=50, loop=0, optimize=True)
```

## Common Image Sizes

| Use case | Size (px) |
|----------|-----------|
| Open Graph / social share | 1200 × 630 |
| Twitter card | 1200 × 675 |
| Instagram post | 1080 × 1080 |
| Instagram story | 1080 × 1920 |
| YouTube thumbnail | 1280 × 720 |
| Favicon | 32 × 32 or 64 × 64 |
| App icon | 512 × 512 |

## Quality Rules

- **Always set DPI**: `plt.savefig(..., dpi=150)` minimum
- **Use tight layout**: `bbox_inches='tight'` to avoid clipping
- **Label everything**: axes, legends, titles on charts
- **Consistent palette**: pick 3–5 colors and reuse them
- **Check contrast**: text must be readable against background
- **Save PNG for graphics** (lossless), **JPEG for photos** (smaller)
