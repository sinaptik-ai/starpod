"""Extract tables from PDF files to CSV or Excel.

Usage:
    python extract_tables.py input.pdf [--output tables.xlsx] [--format csv|xlsx]

Examples:
    python extract_tables.py report.pdf
    python extract_tables.py report.pdf --output data.xlsx
    python extract_tables.py report.pdf --format csv --output table
"""

import argparse
import sys
from pathlib import Path

try:
    import pdfplumber
except ImportError:
    print("Error: pdfplumber required. Install: pip install pdfplumber", file=sys.stderr)
    sys.exit(1)


def extract_tables(pdf_path: str, output: str = None, fmt: str = "xlsx") -> None:
    path = Path(pdf_path)
    if not path.exists():
        print(f"Error: {pdf_path} does not exist", file=sys.stderr)
        sys.exit(1)

    all_tables = []
    with pdfplumber.open(path) as pdf:
        for i, page in enumerate(pdf.pages):
            tables = page.extract_tables()
            for j, table in enumerate(tables):
                if table and len(table) > 1:
                    all_tables.append({
                        "page": i + 1,
                        "index": j + 1,
                        "headers": table[0],
                        "rows": table[1:],
                    })

    if not all_tables:
        print("No tables found in PDF.")
        return

    print(f"Found {len(all_tables)} table(s) across {len(set(t['page'] for t in all_tables))} page(s)")

    try:
        import pandas as pd
    except ImportError:
        # Fallback: print tables as text
        for t in all_tables:
            print(f"\n--- Page {t['page']}, Table {t['index']} ---")
            print("\t".join(str(h) for h in t["headers"]))
            for row in t["rows"]:
                print("\t".join(str(c) for c in row))
        return

    dfs = []
    for t in all_tables:
        df = pd.DataFrame(t["rows"], columns=t["headers"])
        df.attrs["source"] = f"Page {t['page']}, Table {t['index']}"
        dfs.append(df)

    if fmt == "csv":
        base = output or "table"
        for i, df in enumerate(dfs):
            filename = f"{base}_{i+1}.csv" if len(dfs) > 1 else f"{base}.csv"
            df.to_csv(filename, index=False)
            print(f"  Saved: {filename} ({len(df)} rows)")
    else:
        filename = output or "tables.xlsx"
        with pd.ExcelWriter(filename, engine="openpyxl") as writer:
            for i, df in enumerate(dfs):
                sheet = f"Page{all_tables[i]['page']}_Table{all_tables[i]['index']}"
                df.to_excel(writer, sheet_name=sheet[:31], index=False)
        print(f"  Saved: {filename}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Extract tables from PDF to CSV/Excel")
    parser.add_argument("input", help="Input PDF file")
    parser.add_argument("--output", "-o", help="Output filename")
    parser.add_argument("--format", "-f", choices=["csv", "xlsx"], default="xlsx", help="Output format (default: xlsx)")
    args = parser.parse_args()
    extract_tables(args.input, args.output, args.format)
