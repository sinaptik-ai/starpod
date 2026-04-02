---
name: data-analysis
description: "Use this skill when the user asks to analyze data from CSV, JSON, Excel, or database exports — including exploring datasets, computing statistics, creating visualizations, finding patterns, cleaning data, or building dashboards. Trigger whenever the user provides a data file and wants insights, charts, or transformations."
version: 0.1.0
compatibility: "Requires Python 3. Install deps: pip install pandas matplotlib seaborn"
---

# Data Analysis

## Workflow

1. **Load & inspect** — read data, check shape, types, nulls
2. **Clean** — handle missing values, fix types, remove duplicates
3. **Explore** — summary stats, distributions, correlations
4. **Analyze** — answer the specific question
5. **Visualize** — create clear, labeled charts
6. **Report** — summarize findings in plain language

## Quick Start: Data Profiling

```bash
python scripts/profile.py data.csv                    # print profile to stdout
python scripts/profile.py data.xlsx --output report.md  # save to file
python scripts/profile.py data.xlsx --sheet "Sales"     # specific sheet
```

The profiler auto-detects file format and generates: row/column counts, types, null percentages, numeric statistics, and top categorical values.

## Loading Data

```python
import pandas as pd

# Auto-detect format
df = pd.read_csv("data.csv")
df = pd.read_excel("data.xlsx")
df = pd.read_json("data.json")
df = pd.read_csv("data.tsv", sep="\t")

# Handle encoding issues
df = pd.read_csv("data.csv", encoding="latin-1")

# Large files — read in chunks
for chunk in pd.read_csv("large.csv", chunksize=10000):
    process(chunk)
```

## Inspection

```python
df.shape                    # (rows, cols)
df.dtypes                   # column types
df.head(10)                 # first 10 rows
df.describe()               # numeric statistics
df.describe(include='all')  # include categorical
df.isnull().sum()           # missing values per column
df.nunique()                # unique values per column
df.duplicated().sum()       # duplicate rows
```

## Cleaning

```python
# Drop duplicates
df = df.drop_duplicates()

# Handle missing values
df['col'].fillna(df['col'].median(), inplace=True)  # fill with median
df = df.dropna(subset=['critical_col'])               # drop rows missing critical data

# Fix types
df['date'] = pd.to_datetime(df['date'])
df['amount'] = pd.to_numeric(df['amount'], errors='coerce')
df['category'] = df['category'].astype('category')

# Clean strings
df['name'] = df['name'].str.strip().str.lower()

# Rename columns
df.columns = df.columns.str.strip().str.lower().str.replace(' ', '_')
```

## Analysis Patterns

### Group and aggregate
```python
df.groupby('category')['revenue'].agg(['sum', 'mean', 'count'])
df.groupby(['year', 'region']).agg(
    total_sales=('sales', 'sum'),
    avg_price=('price', 'mean'),
    n_orders=('order_id', 'count')
).reset_index()
```

### Time series
```python
df['date'] = pd.to_datetime(df['date'])
df.set_index('date', inplace=True)
monthly = df.resample('M')['value'].sum()
rolling = df['value'].rolling(window=7).mean()
```

### Pivot tables
```python
pivot = df.pivot_table(
    values='revenue',
    index='region',
    columns='quarter',
    aggfunc='sum',
    margins=True
)
```

### Correlations
```python
corr = df[['price', 'quantity', 'revenue', 'rating']].corr()
```

## Visualization

### Setup
```python
import matplotlib.pyplot as plt
import seaborn as sns

sns.set_theme(style="whitegrid")
plt.rcParams['figure.figsize'] = (10, 6)
plt.rcParams['figure.dpi'] = 150
```

### Bar chart
```python
fig, ax = plt.subplots()
data = df.groupby('category')['revenue'].sum().sort_values(ascending=False)
data.plot(kind='bar', ax=ax, color='#1E2761')
ax.set_title('Revenue by Category', fontsize=14, fontweight='bold')
ax.set_xlabel('')
ax.set_ylabel('Revenue ($)')
ax.bar_label(ax.containers[0], fmt='${:,.0f}')
plt.xticks(rotation=45, ha='right')
plt.tight_layout()
plt.savefig('revenue_by_category.png')
```

### Line chart (time series)
```python
fig, ax = plt.subplots()
ax.plot(monthly.index, monthly.values, color='#1E2761', linewidth=2)
ax.fill_between(monthly.index, monthly.values, alpha=0.1, color='#1E2761')
ax.set_title('Monthly Revenue Trend', fontsize=14, fontweight='bold')
ax.set_ylabel('Revenue ($)')
plt.tight_layout()
plt.savefig('trend.png')
```

### Heatmap (correlations)
```python
fig, ax = plt.subplots(figsize=(8, 6))
sns.heatmap(corr, annot=True, fmt='.2f', cmap='RdBu_r', center=0,
            square=True, ax=ax)
ax.set_title('Correlation Matrix', fontsize=14, fontweight='bold')
plt.tight_layout()
plt.savefig('correlations.png')
```

### Histogram / distribution
```python
fig, ax = plt.subplots()
ax.hist(df['value'], bins=30, color='#1E2761', edgecolor='white', alpha=0.8)
ax.axvline(df['value'].mean(), color='#F96167', linestyle='--', label=f"Mean: {df['value'].mean():.1f}")
ax.legend()
ax.set_title('Value Distribution', fontsize=14, fontweight='bold')
plt.tight_layout()
plt.savefig('distribution.png')
```

### Scatter plot
```python
fig, ax = plt.subplots()
ax.scatter(df['x'], df['y'], alpha=0.5, s=20, color='#1E2761')
ax.set_title('X vs Y', fontsize=14, fontweight='bold')
ax.set_xlabel('X')
ax.set_ylabel('Y')
plt.tight_layout()
plt.savefig('scatter.png')
```

## Visualization Rules

- **Always label axes and title** — no unlabeled charts
- **Use consistent colors** — pick a palette and stick with it
- **Annotate key values** — label the most important data points
- **Save at 150+ DPI** — `plt.savefig('chart.png', dpi=150, bbox_inches='tight')`
- **Choose the right chart**: bar for comparison, line for trends, scatter for relationships, histogram for distributions, heatmap for correlations
- **Sort bar charts** — almost always descending by value
- **Limit categories** — show top 10, group the rest as "Other"

## Exporting Results

```python
# To Excel with formatting
with pd.ExcelWriter('report.xlsx', engine='openpyxl') as writer:
    summary.to_excel(writer, sheet_name='Summary')
    details.to_excel(writer, sheet_name='Details')

# To CSV
df.to_csv('output.csv', index=False)

# To markdown table (for reports)
print(df.to_markdown(index=False))
```
