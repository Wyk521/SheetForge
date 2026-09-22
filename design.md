# Design — SheetForge

This is the locked visual system for the SheetForge desktop app. It adapts the design DNA of Hallmark’s “The Cascadia Nightjar” example without copying its content or ticket layout literally.

## Genre

Editorial utility: printed-ticket discipline applied to a dense data workbench.

## Macrostructure family

- App pages: **Route Workbench** — the three application pages form a rail-like route; each page remains a functional work surface rather than a marketing section.
- Navigation: itinerary rail with three labelled stations; active state is a vermilion validation mark.
- Persistent action area: detachable ticket stub with output target, real metrics, progress, and one primary execution action.
- Dialogs: single paper surface, never card-in-card.

## Theme

- `--color-paper` `oklch(93.5% 0.016 78)`
- `--color-paper-2` `oklch(91% 0.020 76)`
- `--color-paper-3` `oklch(88% 0.024 72)`
- `--color-ink` `oklch(26% 0.030 252)`
- `--color-ink-2` `oklch(34% 0.028 250)`
- `--color-rule` `oklch(78% 0.020 70)`
- `--color-accent` `oklch(48% 0.190 32)`
- `--color-focus` `oklch(50% 0.140 252)`

The accent is a validation signal, not a surface colour. Sea-teal is reserved for route/progress structure.

## Typography

- Display: Bodoni/Didot with Songti fallbacks, weight 700, roman only.
- Body: Microsoft YaHei UI / PingFang SC / Noto Sans CJK SC, weight 400.
- Mono: Cascadia Mono / JetBrains Mono, for route numbers, metrics, times, and technical labels.
- Display tracking: `-0.02em`; compact data uses tabular numerals.

## Spacing

4-point named scale from `--space-3xs` through `--space-3xl`. Components consume tokens only.

## Motion

- Button press: 120 ms transform.
- Page/route state: 220 ms opacity and transform.
- Progress remains functional.
- Reduced motion: all spatial motion collapses to ≤ 150 ms.

## Microinteractions stance

- Silent success when the visible state already changed.
- Immediate focus rings; no animated focus.
- Hover styles exist only for fine pointers and always have keyboard equivalents.

## CTA voice

- Primary action: night-ink fill, paper text, squared ticket geometry.
- Secondary action: paper fill with printed hairline.
- Vermilion is reserved for active route nodes, warnings, and validation marks.

## Per-page allowances

- App pages use no decorative imagery; the work surface is the visual proof.
- Tables may scroll horizontally at desktop minimum width, but the app shell itself must not.
- Dense controls may use the project’s compact desktop sizing; touch-only 44 px rules do not override the existing Tauri minimum window.

## What pages MUST share

- Route navigation, paper/ink palette, display/body/mono roles, ticket-edge rules, and bottom execution stub.
- Page headings are stacked, roman, and left-biased.
- Buttons and inputs keep constant border geometry across states.

## What pages MAY differ on

- Data source page may use a larger empty-state receiving area.
- Merge page may use an asymmetric control/workbench split.
- Preview page may give the table nearly the full paper width.

## Exports

### tokens.css

The canonical implementation is [`tokens.css`](tokens.css).

### Tailwind v4 mapping

```css
@theme {
  --color-paper: oklch(93.5% 0.016 78);
  --color-ink: oklch(26% 0.030 252);
  --color-accent: oklch(48% 0.190 32);
  --font-display: "Bodoni MT", "Songti SC", serif;
  --font-body: "Microsoft YaHei UI", "PingFang SC", sans-serif;
  --spacing-md: 1rem;
  --ease-out: cubic-bezier(0.16, 1, 0.3, 1);
}
```

### DTCG mapping

```json
{
  "color": {
    "paper": { "$value": "oklch(93.5% 0.016 78)", "$type": "color" },
    "ink": { "$value": "oklch(26% 0.030 252)", "$type": "color" },
    "accent": { "$value": "oklch(48% 0.190 32)", "$type": "color" }
  },
  "space": { "md": { "$value": "1rem", "$type": "dimension" } }
}
```

### shadcn/ui mapping

```css
:root {
  --background: 93.5% 0.016 78;
  --foreground: 26% 0.030 252;
  --primary: 26% 0.030 252;
  --primary-foreground: 93.5% 0.016 78;
  --muted: 91% 0.020 76;
  --muted-foreground: 46% 0.024 248;
  --border: 78% 0.020 70;
  --input: 78% 0.020 70;
  --ring: 50% 0.140 252;
  --radius: 4px;
}
```
