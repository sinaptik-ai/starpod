---
name: web-research
description: "Use this skill when the user asks to research a topic, find information online, compare options, investigate a question, or compile findings from multiple sources. Trigger for any task that requires searching the web, reading articles, and synthesizing findings into a structured answer."
version: 0.1.0
compatibility: "Requires web search and fetch capabilities"
---

# Web Research

## Methodology

Follow this structured approach for any research task:

### 1. Scope the question
Before searching, clarify:
- What specifically does the user need to know?
- What format should the answer take? (summary, comparison table, list, report)
- How deep should the research go? (quick answer vs. comprehensive analysis)

### 2. Search strategy
- Start with **2–3 targeted queries** using different phrasings
- Use specific terms, not vague ones: "React server components vs client components performance 2025" beats "React performance"
- For comparisons, search each option independently before comparing
- For recent topics, include the year in queries
- If initial results are thin, broaden terms or try adjacent queries

### 3. Source evaluation
Prioritize sources in this order:
1. **Official documentation** — most reliable for technical topics
2. **Primary sources** — research papers, official reports, announcements
3. **Expert analysis** — recognized experts, reputable publications
4. **Community consensus** — Stack Overflow, GitHub discussions (check vote counts and recency)
5. **News articles** — for current events, cross-reference multiple outlets

**Red flags**: outdated dates, no author attribution, SEO-farm patterns, contradicted by official sources.

### 4. Synthesize findings
- Lead with the direct answer, then supporting evidence
- Note conflicting information across sources and which is more credible
- Include source URLs for key claims
- Flag uncertainty: "Based on available data..." or "Sources disagree on..."
- Distinguish facts from opinions

### 5. Present results

**For quick questions**: Direct answer in 2–3 sentences with source link.

**For comparisons**:
| Criteria | Option A | Option B |
|----------|----------|----------|
| Key metric | Value | Value |
| Pros | ... | ... |
| Cons | ... | ... |

**For deep research**: Structure as:
1. **Summary** — 2–3 sentence answer
2. **Key findings** — bulleted list of important points
3. **Analysis** — detailed discussion organized by theme
4. **Sources** — numbered list of references used

## Common Research Patterns

### Product/tool comparison
1. Search for each product independently
2. Search for "X vs Y" comparison articles
3. Check official pricing pages and feature lists
4. Look for user reviews on neutral platforms
5. Compile into comparison table

### Technical question
1. Check official documentation first
2. Search for the specific error message or API
3. Look at GitHub issues and Stack Overflow
4. Check release notes if version-specific

### Current events / news
1. Search multiple news sources
2. Check the date — prefer last 48 hours for breaking news
3. Look for the primary source (press release, official statement)
4. Cross-reference facts across 2+ outlets

### How-to / tutorial
1. Find official docs or getting-started guides
2. Check for community tutorials with recent dates
3. Verify steps actually work (versions matter)
4. Prefer tutorials that explain *why*, not just *what*

## Output Quality Rules

- **Always cite sources** — include URLs for every factual claim
- **Always note dates** — "As of March 2025, ..." for time-sensitive info
- **Never fabricate** — if you can't find it, say so
- **Quantify when possible** — "65% of developers prefer X" beats "most developers prefer X"
- **Acknowledge limits** — if research was shallow or sources were scarce, say so
