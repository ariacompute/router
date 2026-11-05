<!-- impeccable:product-schema 1 -->
# Aria Router — product & design contract

## Positioning

Aria Router is a **local-first operations console for an OpenAI-compatible
inference gateway**. It is not a chat playground and not a marketing site.
Operators use it to inspect routing decisions, manage keys, watch spend and
validate configuration while the gateway is running.

## Audience

- SRE / platform engineers running `aria-router serve`.
- Model owners comparing competence tiers, cost and routing evidence.
- Administrators managing local accounts and API keys.

## Jobs to be done

1. See whether the gateway is healthy and which decisions it is making.
2. Inspect replayed decisions, execution evidence and trust-tier drafts.
3. Compare cost / traffic share per model and competence tier.
4. Change configuration explicitly and understand that the change is immediate.
5. Manage API keys, spend limits and local users.

## Design principles

- **Monochrome**: neutral ink on paper; functional colours only for state.
- **Square**: zero corner radii everywhere (including chips and buttons).
- **Flat**: no gradients, no elevation, hairline borders only.
- **Legible**: minimum 12px body text; AA contrast for all text tokens.
- **Quiet motion**: transitions/animations may only use `transform` and `opacity`.
- **Local**: no web-font CDN, no external runtime assets, no telemetry.
- **Honest**: empty states say "no data"; estimates are labelled estimates.

## Tokens (source of truth: `dashboard/src/index.css`)

| Token | Contract |
|-------|----------|
| `--page-bg` / `--panel-bg` | white in light, near-black in dark |
| `--text-main` / `--text-muted` / `--text-soft` | AA contrast against both page and panel |
| `--radius*` (incl. `--radius-pill`) | `0` |
| `--font-sans` / `--font-display` / `--font-label` | local system stacks only |
| `--font-size-min` | `12px` |
| `--accent` | neutral ink, inverted in dark |
| `--shadow*` | `none` |

## Detection modes

| Mode | Source | Element-level rules |
|------|--------|--------------------|
| `static` | `dashboard/src/**/*.css`, `dashboard/index.html` | CSS + HTML rules |
| `dom` | jsdom render of every route in both themes | DOM rules (TSX is not statically parsed) |
| `url` | live preview + Chrome | full rendering rules; **skipped** (not passed) without a browser |

TSX/JSX is deliberately excluded from the static engine: impeccable's static
parser is HTML-oriented. Element-level coverage comes from `dom` and `url`.

## Page matrix

`/login`, `/register`, `/`, `/account`, `/cost`, `/cost-competence`,
`/model-performance`, `/trust`, `/keys`, `/users` (admin), `/config`,
`/topology`, `/providers`, `/replay`, `/playground` — **15 routes ×
light/dark**. There is no i18n dimension; UI copy is English.

## Non-goals

- No gamification, gradients, decorative illustration or marketing hero.
- No change to API contracts, SSE streaming, route response headers or
  session handling as part of a visual audit.
- No fabricated audit result: a mode that cannot run must be recorded as
  `skipped`.
