# Design Language

## Colors

3-color palette derived from the suit sprites:

| Token    | Hex       | Usage                         |
| -------- | --------- | ----------------------------- |
| `paper`  | `#ffffff` | Backgrounds, card faces       |
| `ink`    | `#1a1a1a` | Text, borders, outlines       |
| `accent` | `#c0392b` | Highlights, errors, red suits |

No other colors. Use opacity on `ink` for muted/secondary text.

## Typography

- **`Press Start 2P`** — all text, everywhere
- No rounded corners (`border-radius: 0`)
- `image-rendering: pixelated` on all images

## Components

- **Buttons**: `2px solid ink` border, `3px 3px 0 ink` shadow. Press effect via `translate` on hover/active.
- **Cards**: white face, `2px ink` border, `3px shadow`. Red suits use accent, black suits use ink. Card backs: ink crosshatch on white.
- **Containers**: `3px ink` border, `6px 6px 0 ink` shadow. White fill.
- **Inputs**: `2px ink` border, focus ring uses accent color with shadow.
- **Table**: white surface, `6px ink` border, `8px 8px 0 ink` shadow.
