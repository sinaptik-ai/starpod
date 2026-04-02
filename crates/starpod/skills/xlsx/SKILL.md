---
name: xlsx
description: "Use this skill any time a spreadsheet file is the primary input or output — opening, reading, editing, or creating .xlsx, .xlsm, .csv, or .tsv files. Trigger for data formatting, formula creation, charting, financial modeling, or converting between tabular formats. Also trigger for cleaning messy tabular data into proper spreadsheets."
version: 0.1.0
compatibility: "Requires: pip install openpyxl pandas xlsxwriter. System: apt install libreoffice (for formula recalculation)"
---

# XLSX Skill

## Decision Tree

| Task | Tool |
|------|------|
| Data analysis, bulk operations | pandas |
| Formulas, formatting, styles | openpyxl |
| High-performance chart creation | xlsxwriter |
| Read calculated values (no formulas) | `openpyxl` with `data_only=True` |

## CRITICAL: Use Excel Formulas, Not Hardcoded Values

```python
# ❌ WRONG — calculating in Python
total = df['Sales'].sum()
sheet['B10'] = total

# ✅ CORRECT — let Excel calculate
sheet['B10'] = '=SUM(B2:B9)'
sheet['C5'] = '=(C4-C2)/C2'
sheet['D20'] = '=AVERAGE(D2:D19)'
```

Always use Excel formulas so spreadsheets remain dynamic and updateable.

## Reading & Analyzing

### pandas (data analysis)
```python
import pandas as pd

df = pd.read_excel('file.xlsx')                          # first sheet
all_sheets = pd.read_excel('file.xlsx', sheet_name=None)  # all sheets as dict

df.describe()                         # statistics
df.groupby('Category')['Amount'].sum()  # aggregation
df.to_excel('output.xlsx', index=False)
```

### openpyxl (preserve formulas/formatting)
```python
from openpyxl import load_workbook

wb = load_workbook('file.xlsx')
sheet = wb.active
for row in sheet.iter_rows(min_row=2, values_only=True):
    print(row)
```

## Creating New Spreadsheets

### openpyxl with formulas and formatting
```python
from openpyxl import Workbook
from openpyxl.styles import Font, PatternFill, Alignment, numbers

wb = Workbook()
ws = wb.active
ws.title = "Revenue Model"

# Headers
headers = ["Quarter", "Revenue", "COGS", "Gross Profit", "Margin"]
for col, h in enumerate(headers, 1):
    cell = ws.cell(row=1, column=col, value=h)
    cell.font = Font(bold=True, color="FFFFFF")
    cell.fill = PatternFill("solid", fgColor="333333")
    cell.alignment = Alignment(horizontal="center")

# Data with formulas
quarters = ["Q1", "Q2", "Q3", "Q4"]
revenues = [120000, 145000, 168000, 192000]
cogs_pct = 0.35

for i, (q, rev) in enumerate(zip(quarters, revenues), 2):
    ws.cell(row=i, column=1, value=q)
    ws.cell(row=i, column=2, value=rev).number_format = '$#,##0'
    ws.cell(row=i, column=3).value = f'=B{i}*{cogs_pct}'
    ws.cell(row=i, column=3).number_format = '$#,##0'
    ws.cell(row=i, column=4).value = f'=B{i}-C{i}'
    ws.cell(row=i, column=4).number_format = '$#,##0'
    ws.cell(row=i, column=5).value = f'=D{i}/B{i}'
    ws.cell(row=i, column=5).number_format = '0.0%'

# Totals row
total_row = len(quarters) + 2
ws.cell(row=total_row, column=1, value="Total").font = Font(bold=True)
for col in [2, 3, 4]:
    cell = ws.cell(row=total_row, column=col)
    cell.value = f'=SUM({chr(64+col)}2:{chr(64+col)}{total_row-1})'
    cell.font = Font(bold=True)
    cell.number_format = '$#,##0'

# Column widths
for col_letter, width in [("A", 12), ("B", 15), ("C", 15), ("D", 15), ("E", 12)]:
    ws.column_dimensions[col_letter].width = width

wb.save("revenue_model.xlsx")
```

### xlsxwriter (charts)
```python
import xlsxwriter

wb = xlsxwriter.Workbook("chart_report.xlsx")
ws = wb.add_worksheet("Sales")

data = [["Month", "Sales"], ["Jan", 5000], ["Feb", 6200], ["Mar", 7100], ["Apr", 6800]]
for r, row in enumerate(data):
    for c, val in enumerate(row):
        ws.write(r, c, val)

chart = wb.add_chart({"type": "column"})
chart.add_series({
    "name": "Sales",
    "categories": ["Sales", 1, 0, len(data)-1, 0],
    "values": ["Sales", 1, 1, len(data)-1, 1],
    "fill": {"color": "#1E2761"},
})
chart.set_title({"name": "Monthly Sales"})
chart.set_size({"width": 600, "height": 400})
ws.insert_chart("D2", chart)

wb.close()
```

## Editing Existing Files

```python
from openpyxl import load_workbook

wb = load_workbook('existing.xlsx')
ws = wb['Sheet1']

ws['A1'] = 'Updated Value'
ws.insert_rows(2)               # insert row at position 2
ws.delete_cols(3)               # delete column C
new_ws = wb.create_sheet('Summary')
new_ws['A1'] = '=Sheet1!B10'    # cross-sheet reference

wb.save('modified.xlsx')
```

**Warning**: `load_workbook('f.xlsx', data_only=True)` reads calculated values but **permanently loses formulas** if saved.

## Financial Model Standards

### Color coding (industry standard)
| Color | Meaning |
|-------|---------|
| **Blue text** (0,0,255) | Hardcoded inputs / assumptions |
| **Black text** (0,0,0) | All formulas and calculations |
| **Green text** (0,128,0) | Cross-sheet links within workbook |
| **Red text** (255,0,0) | External links to other files |
| **Yellow background** | Key assumptions needing attention |

### Number formatting
| Type | Format |
|------|--------|
| Currency | `$#,##0` with units in header ("Revenue ($mm)") |
| Years | Text format (avoid "2,024") |
| Percentages | `0.0%` (one decimal) |
| Multiples | `0.0x` (EV/EBITDA, P/E) |
| Negatives | Parentheses `(123)` not minus `-123` |
| Zeros | Display as `-` via `$#,##0;($#,##0);"-"` |

### Formula rules
- Place ALL assumptions in separate cells — never hardcode in formulas
- Use `=B5*(1+$B$6)` not `=B5*1.05`
- Document sources for hardcoded values: "Source: Company 10-K, FY2024, Page 45"
- Test formulas on 2–3 cells before applying broadly

## Recalculating Formulas

openpyxl writes formulas as strings without calculated values. Use the bundled script:

```bash
python scripts/recalc.py output.xlsx [timeout_seconds]
```

The script:
- Installs a LibreOffice macro on first run
- Recalculates all formulas in all sheets
- Scans every cell for Excel errors (#REF!, #DIV/0!, etc.)
- Returns JSON with error locations and counts

```json
{
  "status": "success",
  "total_errors": 0,
  "total_formulas": 42,
  "error_summary": {}
}
```

If `status` is `"errors_found"`, fix the issues and re-run.

## Common Pitfalls

- Cell indices are **1-based** in openpyxl (row=1, column=1 = A1)
- `data_only=True` + save = formulas permanently lost
- For large files: `read_only=True` for reading, `write_only=True` for writing
- Specify dtypes to avoid pandas inference: `pd.read_excel('f.xlsx', dtype={'id': str})`
- Always verify cross-sheet references use correct format: `Sheet1!A1`
