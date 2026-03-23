# Starpod

## Design Context

### Users
Consumer product — Starpod is a personal AI assistant platform designed for a broad audience. Users interact through a chat interface (web + Telegram) and expect a polished, approachable experience. The interface should feel immediately familiar to anyone who has used modern AI chat products, while rewarding power users with keyboard shortcuts and dense information display.

### Brand Personality
**Minimal, technical, sharp.** Starpod speaks through restraint — every pixel earns its place. The aesthetic signals competence and precision without being cold. Think: a well-made instrument that feels good in your hands.

### Aesthetic Direction
- **Visual tone**: Dark-first, high-contrast, developer-grade polish. Pure grayscale palette with silver accent (#C0C0C0). Sharp corners (0px border-radius). Space Grotesk for brand, monospace for technical elements, system sans-serif for prose.
- **References**: Linear (crisp dark UI, keyboard-first, fast transitions), Raycast (speed, polish), Claude.ai/ChatGPT (conversational AI patterns), Vercel/Stripe (premium developer tools, elegant typography).
- **Anti-references**: Cluttered dashboards, gradients-everywhere SaaS, bubbly/playful UI, skeleton screens that flash. Nothing that feels slow or indecisive.
- **Theme**: Dark mode only. Background `#0A0A0A`, surface `#111111`, accent `#C0C0C0` (silver).

### Design Principles
1. **Precision over decoration** — No ornamental elements. Every border, shadow, and color change communicates state or hierarchy. If it doesn't serve a function, remove it.
2. **Speed is a feature** — Transitions are fast (150-300ms), interactions feel instant, layout never shifts unexpectedly. The UI should feel like it's keeping up with thought.
3. **Dense but breathable** — Pack information tightly but use consistent spacing and clear hierarchy so nothing feels cramped. Whitespace is structural, not decorative.
4. **Quiet until needed** — Status indicators, errors, and tool feedback appear contextually and disappear gracefully. The default state is calm.
5. **Keyboard-native, touch-ready** — Design for keyboard-first interaction with proper focus management, but ensure all touch targets meet 44px minimum and mobile layouts work standalone.

### Tech Stack
- React 19, Vite, Tailwind CSS 4 (custom `@theme` tokens)
- `marked` for GFM markdown rendering
- CSS custom properties for all colors/spacing (see `web/src/style.css`)
- No external component library — all components are custom
- Font stack: system sans-serif (`-apple-system` → `Inter`) + `JetBrains Mono` for code/technical + `Space Grotesk` for brand/display

### Color System
| Token | Hex | Role |
|-------|-----|------|
| `--color-bg` | `#0A0A0A` | Page background |
| `--color-surface` | `#111111` | Cards, sidebar |
| `--color-elevated` | `#151515` | Hover, modals |
| `--color-border-main` | `#222222` | Primary borders |
| `--color-border-subtle` | `#1A1A1A` | Dividers |
| `--color-dim` | `#555555` | Disabled text |
| `--color-muted` | `#666666` | Secondary text |
| `--color-secondary` | `#888888` | Tertiary text |
| `--color-primary` | `#E8E8E8` | Main text |
| `--color-accent` | `#C0C0C0` | CTA, active states (silver) |
| `--color-ok` | `#22c55e` | Success |
| `--color-err` | `#ef4444` | Error |
| `--color-warn` | `#eab308` | Warning |

### Key Dimensions
- Sidebar: 280px
- Max content width: 740px
- Header height: 48px
- Mobile breakpoint: 768px
- Border radius: 0px (sharp corners everywhere), 50%/999px (circles/pills only)
- Transition: `0.15s–0.3s cubic-bezier(0.4, 0, 0.2, 1)`
