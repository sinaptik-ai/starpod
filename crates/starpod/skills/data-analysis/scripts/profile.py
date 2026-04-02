"""Generate a data profile report from CSV, Excel, or JSON files.

Produces summary statistics, type inference, missing values, and
basic distribution info without manual exploration.

Usage:
    python profile.py data.csv [--output report.md]
    python profile.py data.xlsx --sheet "Sales"

Examples:
    python profile.py customers.csv
    python profile.py financials.xlsx --output profile.md
"""

import argparse
import sys
from pathlib import Path

try:
    import pandas as pd
except ImportError:
    print("Error: pandas required. Install: pip install pandas openpyxl", file=sys.stderr)
    sys.exit(1)


def load_data(filepath: str, sheet: str = None) -> pd.DataFrame:
    path = Path(filepath)
    suffix = path.suffix.lower()

    if suffix == ".csv":
        return pd.read_csv(path)
    elif suffix == ".tsv":
        return pd.read_csv(path, sep="\t")
    elif suffix in (".xlsx", ".xls", ".xlsm"):
        return pd.read_excel(path, sheet_name=sheet or 0)
    elif suffix == ".json":
        return pd.read_json(path)
    elif suffix == ".parquet":
        return pd.read_parquet(path)
    else:
        return pd.read_csv(path)


def profile(df: pd.DataFrame, name: str = "dataset") -> str:
    lines = [f"# Data Profile: {name}\n"]

    # Overview
    lines.append(f"## Overview")
    lines.append(f"- **Rows**: {len(df):,}")
    lines.append(f"- **Columns**: {len(df.columns)}")
    lines.append(f"- **Memory**: {df.memory_usage(deep=True).sum() / 1024 / 1024:.1f} MB")
    lines.append(f"- **Duplicates**: {df.duplicated().sum():,} rows")
    lines.append("")

    # Column summary
    lines.append("## Columns\n")
    lines.append("| Column | Type | Non-Null | Null % | Unique | Sample |")
    lines.append("|--------|------|----------|--------|--------|--------|")

    for col in df.columns:
        dtype = str(df[col].dtype)
        non_null = df[col].notna().sum()
        null_pct = f"{df[col].isna().mean() * 100:.1f}%"
        unique = df[col].nunique()
        sample = str(df[col].dropna().iloc[0])[:30] if non_null > 0 else "N/A"
        lines.append(f"| {col} | {dtype} | {non_null:,} | {null_pct} | {unique:,} | {sample} |")
    lines.append("")

    # Numeric statistics
    numeric_cols = df.select_dtypes(include="number").columns
    if len(numeric_cols) > 0:
        lines.append("## Numeric Statistics\n")
        lines.append("| Column | Mean | Std | Min | 25% | 50% | 75% | Max |")
        lines.append("|--------|------|-----|-----|-----|-----|-----|-----|")
        desc = df[numeric_cols].describe()
        for col in numeric_cols:
            lines.append(
                f"| {col} | {desc[col]['mean']:.2f} | {desc[col]['std']:.2f} | "
                f"{desc[col]['min']:.2f} | {desc[col]['25%']:.2f} | {desc[col]['50%']:.2f} | "
                f"{desc[col]['75%']:.2f} | {desc[col]['max']:.2f} |"
            )
        lines.append("")

    # Categorical columns
    cat_cols = df.select_dtypes(include=["object", "category"]).columns
    if len(cat_cols) > 0:
        lines.append("## Categorical Columns\n")
        for col in cat_cols[:10]:
            top = df[col].value_counts().head(5)
            lines.append(f"### {col} (top 5 of {df[col].nunique()} unique)")
            for val, count in top.items():
                pct = count / len(df) * 100
                lines.append(f"- `{val}`: {count:,} ({pct:.1f}%)")
            lines.append("")

    # Date columns
    date_cols = df.select_dtypes(include="datetime").columns
    if len(date_cols) > 0:
        lines.append("## Date Columns\n")
        for col in date_cols:
            lines.append(f"### {col}")
            lines.append(f"- Range: {df[col].min()} to {df[col].max()}")
            lines.append(f"- Span: {(df[col].max() - df[col].min()).days} days")
            lines.append("")

    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate data profile report")
    parser.add_argument("input", help="Input data file (CSV, XLSX, JSON, Parquet)")
    parser.add_argument("--output", "-o", help="Output markdown file (default: print to stdout)")
    parser.add_argument("--sheet", help="Sheet name for Excel files")
    args = parser.parse_args()

    df = load_data(args.input, args.sheet)
    report = profile(df, Path(args.input).name)

    if args.output:
        Path(args.output).write_text(report)
        print(f"Profile saved to {args.output}")
    else:
        print(report)
