# Starpod

## Design Context

### Users
Consumer product — Starpod is a personal AI assistant platform designed for a broad audience. Users interact through a chat interface (web + Telegram) and expect a polished, approachable experience. The interface should feel immediately familiar to anyone who has used modern AI chat products, while rewarding power users with keyboard shortcuts and dense information display.

### Brand Personality
**Minimal, technical, sharp.** Starpod speaks through restraint — every pixel earns its place. The aesthetic signals competence and precision without being cold. Think: a well-made instrument that feels good in your hands.

### Aesthetic Direction
- **Visual tone**: Dark-first, high-contrast, developer-grade polish. Monochrome zinc palette with blue accent. Monospace typography for technical elements, system sans-serif for prose.
- **References**: Linear (crisp dark UI, keyboard-first, fast transitions), Raycast (speed, polish), Claude.ai/ChatGPT (conversational AI patterns), Vercel/Stripe (premium developer tools, elegant typography).
- **Anti-references**: Cluttered dashboards, gradients-everywhere SaaS, bubbly/playful UI, skeleton screens that flash. Nothing that feels slow or indecisive.
- **Theme**: Dark mode only. Background `#09090b`, surface `#111114`, accent `#3b82f6`.

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
- Font stack: system sans-serif (`-apple-system` → `Inter`) + `JetBrains Mono` for code/technical

### Color System
| Token | Hex | Role |
|-------|-----|------|
| `--color-bg` | `#09090b` | Page background |
| `--color-surface` | `#111114` | Cards, sidebar |
| `--color-elevated` | `#19191e` | Hover, modals |
| `--color-border-main` | `#27272a` | Primary borders |
| `--color-border-subtle` | `#1e1e23` | Dividers |
| `--color-dim` | `#52525b` | Disabled text |
| `--color-muted` | `#71717a` | Secondary text |
| `--color-secondary` | `#a1a1aa` | Tertiary text |
| `--color-primary` | `#e4e4e7` | Main text |
| `--color-accent` | `#3b82f6` | CTA, active states |
| `--color-ok` | `#22c55e` | Success |
| `--color-err` | `#ef4444` | Error |
| `--color-warn` | `#eab308` | Warning |

### Key Dimensions
- Sidebar: 280px
- Max content width: 740px
- Header height: 48px
- Mobile breakpoint: 768px
- Border radius: 8px (cards), 16px (bubbles), 999px (pills)
- Transition: `0.15s–0.3s cubic-bezier(0.4, 0, 0.2, 1)`
