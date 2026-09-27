# Ironflow logo

A solid block with a single channel cut through it: the iron gives the flow its shape. The name is
`Ironflow` in prose and in the logo, `ironflow` for crates, binaries and the CLI.

## Files

| File | What it is |
|---|---|
| `ironflow-logo-light.svg` | Horizontal logo for light backgrounds (primary symbol, ink wordmark) |
| `ironflow-logo-dark.svg` | Horizontal logo for dark backgrounds (dark-theme primary symbol, paper wordmark) |
| `ironflow-logo-black.svg` | One-colour logo, black, for print and single-colour use |
| `ironflow-logo-white.svg` | One-colour logo, white, for photos and coloured backgrounds |
| `ironflow-symbol-light.svg` | Symbol alone for light backgrounds (256 x 256 viewBox) |
| `ironflow-symbol-dark.svg` | Symbol alone for dark backgrounds (256 x 256 viewBox) |
| `ironflow-symbol.svg` | Symbol that follows the system colour scheme, for `<img>` tags outside the dashboard |
| `ironflow-icon.svg` | Square icon: symbol on a rounded dark tile (200 x 200 viewBox) |
| `ironflow-avatar.png` | 512 x 512 avatar: symbol on a full dark square, safe for circular crops |
| `ironflow-banner.png` | 2560 x 640 banner with its own dark background, readable on any theme |
| `ironflow-social.png` | 1280 x 640 card: logo, tagline, a real `ctx.shell` call |
| `favicon/favicon.svg` | Favicon, small-size cut, switches between the light and dark primary with the browser theme |
| `favicon/favicon.ico` | 16, 32 and 48 px favicon for browsers without SVG favicons |
| `favicon/apple-touch-icon.png` | 180 x 180 home-screen icon for iOS |
| `favicon/icon-192.png`, `favicon/icon-512.png` | Web app manifest icons, symbol inside the maskable safe zone |

## What to use where

### README

The root `README.md` uses the header below. GitHub picks the light or dark SVG from the viewer's
theme. GitLab strips `<picture>` and `<source>`, so it shows the `<img>` fallback, the banner, which
reads on both GitLab themes.

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="design/logo/ironflow-logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="design/logo/ironflow-logo-light.svg">
  <img alt="Ironflow" src="design/logo/ironflow-banner.png" width="560">
</picture>
```

Crate READMEs published on crates.io need an absolute URL (it resolves once the file is on `main`):

```html
<img alt="Ironflow" width="560"
     src="https://gitlab.com/thomastartrau/ironflow/-/raw/main/design/logo/ironflow-banner.png">
```

### Dashboard

The dashboard already ships these files in `ironflow-dashboard/public/`:

| Dashboard file | Copy of |
|---|---|
| `favicon.svg` | `favicon/favicon.svg` |
| `logo.svg` | `ironflow-symbol-light.svg` |
| `logo-dark.svg` | `ironflow-symbol-dark.svg` |

`branding.json` points `logoUrl` at `/logo.svg` and `logoDarkUrl` at `/logo-dark.svg`. The sidebar
(28 px) and the sign-in and sign-up pages (36 px) pick the file from the app's resolved theme
through `useBrandLogo()`, so the symbol follows the theme toggle, not the system setting. A custom
`branding.json` that sets only `logoUrl` uses that logo in both themes. When updating the logo,
copy the files again from this folder.

### mdBook

Copy `favicon/favicon.svg` to `docs/book/theme/favicon.svg` and `favicon/icon-192.png` to
`docs/book/theme/favicon.png`; mdBook picks both up without configuration.

### docs.rs

In each crate root, point rustdoc at the PNG files on `main`:

```rust
#![doc(html_logo_url = "https://gitlab.com/thomastartrau/ironflow/-/raw/main/design/logo/favicon/icon-192.png")]
#![doc(html_favicon_url = "https://gitlab.com/thomastartrau/ironflow/-/raw/main/design/logo/favicon/favicon.ico")]
```

### Portfolio (tartrau.fr project page)

- Project logo: `ironflow-icon.svg`, copied to the site as `/logos/ironflow.svg`. It follows the same
  template as the other project logos (200 viewBox, 180 tile, radius 40) and works in the light and
  dark themes without a variant.
- Project image: a real screenshot of the dashboard matches the other project pages best;
  `ironflow-social.png` works as a fallback and as the Open Graph image.

### LinkedIn

- Project section of the profile, or a post about Ironflow: `ironflow-social.png` as the media.
- LinkedIn page logo, if a page is ever created: `ironflow-avatar.png`.

### GitLab

- Project avatar: `ironflow-avatar.png`, in Settings > General > Project avatar (GitLab recommends
  192 x 192 and 200 KB at most; it resizes the file).

### GitHub

- A repository has no icon. Set `ironflow-social.png` in Settings > General > Social preview; it
  shows in link previews on LinkedIn, Slack, X and others.

## Colours

The two blues are the dashboard's `primary` token.

| Name | HEX | OKLCH | Use |
|---|---|---|---|
| Primary | `#007AA7` | `oklch(0.52 0.16 220)` | The symbol on light backgrounds (4.6:1 on `#F6F9FA`) |
| Primary dark | `#0099C8` | `oklch(0.62 0.16 220)` | The symbol on dark backgrounds (6.3:1 on `#020404`) |
| Ink | `#040607` | | Wordmark on light backgrounds |
| Paper | `#E2E9EB` | | Wordmark on dark backgrounds |
| Background dark | `#020404` | | Tiles, avatar, banner, social card |

## Construction

- Symbol: a 192-unit square with 6-unit corners (the near-sharp corners of the dashboard), cut by a
  quarter ring centred on its top-left corner. The quarter disc has a radius of 100 and the channel
  is 22 wide; the dark-background files widen it to 24 because a light shape on a dark ground looks
  bolder. The favicons use a small-size cut (radius 92, channel 34) so the channel still shows at
  16 px.
- Wordmark: Geist SemiBold (SIL Open Font License 1.1) with stylistic set 05, whose serifed `I`
  keeps "Ironflow" from reading "lronflow". Kerned, tracked at -0.5 %, converted to outlines.
- Lockup: the symbol is 1.75 times the cap height, the wordmark is centred on it, and the gap
  between them is 0.56 times the cap height.

## Rules

- Keep clear space around the logo at least as tall as the `o` of the wordmark, and a quarter of the
  symbol's width around the symbol alone.
- Below 120 px wide, use the symbol alone; below 24 px, use the `favicon/` files.
- Don't stretch, rotate or mirror the symbol: the channel always sits in the top-left corner.
- Don't recolour it outside the table above, add effects, retype the wordmark in a font, or put the
  light version on a dark background.
