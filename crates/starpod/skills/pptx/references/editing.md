# PPTX Editing Workflow

For editing existing presentations at the XML level (when python-pptx isn't sufficient).

## Workflow

1. **Analyze** the template visually
   ```bash
   python scripts/thumbnail.py template.pptx
   ```

2. **Unpack** the PPTX
   ```bash
   python scripts/office/unpack.py template.pptx unpacked/
   ```

3. **Explore** the XML structure
   - `unpacked/ppt/presentation.xml` — slide order, sizes
   - `unpacked/ppt/slides/slide1.xml` — slide content
   - `unpacked/ppt/slideMasters/` — master layouts
   - `unpacked/ppt/slideLayouts/` — layout templates
   - `unpacked/ppt/_rels/presentation.xml.rels` — relationships

4. **Edit** XML files directly using the Edit tool

5. **Clean** orphaned files
   ```bash
   python scripts/clean.py unpacked/
   ```

6. **Pack** back into PPTX
   ```bash
   python scripts/office/pack.py unpacked/ output.pptx
   ```

7. **Verify** visually
   ```bash
   python scripts/thumbnail.py output.pptx
   ```

## Key XML Patterns

### Slide structure
```xml
<p:sld>
  <p:cSld>
    <p:spTree>
      <!-- Shape tree: all content goes here -->
      <p:sp>
        <p:txBody>
          <a:p>
            <a:r>
              <a:rPr lang="en-US" sz="2400" b="1"/>
              <a:t>Slide Title</a:t>
            </a:r>
          </a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>
```

### Text formatting
- `sz="2400"` = 24pt (value is in hundredths of a point)
- `b="1"` = bold
- `i="1"` = italic
- `<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>` = color

### Position and size (EMU units)
- 1 inch = 914400 EMU
- 1 cm = 360000 EMU
- `<a:off x="914400" y="457200"/>` = position (1", 0.5")
- `<a:ext cx="7315200" cy="914400"/>` = size (8" x 1")

## Common Tasks

### Duplicate a slide
1. Copy `slideN.xml` and its `.rels` file
2. Add relationship in `presentation.xml.rels`
3. Add `<p:sldId>` in `presentation.xml`
4. Add content type override in `[Content_Types].xml`

### Add an image
1. Place image in `ppt/media/`
2. Add relationship in slide's `.rels` file
3. Add `<p:pic>` element in slide XML
4. Add content type if new format

### Replace text
Use the Edit tool to find and replace `<a:t>` content in slide XML files.
