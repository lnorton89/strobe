# Strobe site

The public website for [Strobe](https://github.com/lnorton89/strobe), built with
Next.js 16 and exported as static files.

```bash
npm install
npm run dev          # http://localhost:3000
npm run build        # static export into out/
npm run lint && npm run typecheck && npm test
npm run test:e2e     # Playwright, against the built out/
```

The design follows the brand guide, [`docs/branding.md`](../docs/branding.md):
the Void/Panel/Border palette with cyan as the active accent, the primary
gradient kept for the mark and a few compact rules, Space Grotesk for
headings, Inter for body copy and JetBrains Mono for anything technical. The
tokens are in `src/app/globals.css` and `tests/unit/palette.test.ts` pins every
text/surface pair against WCAG AA in both themes.

## Nothing is written twice

The site describes the repository it is built inside, and reads it at build
time rather than keeping copies:

- **The current release** is GitHub's latest published one (D557). `npm run
  assets` looks it up once per build (`scripts/sync-release.mts`): GitHub, then
  `git describe --tags`, then naming no number. The browser asks GitHub again on
  each visit (`src/components/Release.tsx`), so a release published since the last
  deploy shows up with no code edit and no deploy. Nothing in the site's source
  names a version; `tests/unit/hardcoded.test.ts` fails if one appears.
- **The frame times on the home page** are read from `docs/BENCHMARKS.md`
  (`src/lib/benchmarks.ts`).

- **`/guide/`, `/compatibility/`, `/architecture/` and `/roadmap/`** render
  `docs/USER_GUIDE.md`, `docs/COMPATIBILITY.md`, `docs/ARCHITECTURE.md` and
  `docs/ROADMAP.md` (`src/lib/repo-docs.ts`). Links between those documents go
  to their pages here; every other repository link goes to GitHub.
- **`/scenes/`** is built from `scenes/*.synScene/scene.json`
  (`src/lib/scenes.ts`). A scene added to the repository appears on the next
  build.
- **Images** (the scene thumbnails, the screenshots in `docs/images/` and the
  icon PNGs from `assets/brand/`) are copied, with thumbnails re-encoded to 640 px
  WebP, into the git-ignored `public/generated/` by `scripts/sync-assets.mjs`,
  which runs before `dev` and `build`.
- **Social cards** are rendered by [Metaplate](https://github.com/freeknet/metaplate)
  into `public/<route>/og-image.jpg` before `dev` and `build`
  (`scripts/render-cards.mts`), one per route, from the copy in
  `src/lib/og/cards.ts`. A card that overflows fails the build. Every page states
  its metadata through `src/lib/og/metadata.ts`.
- **The roadmap's activity feed** is the repository's own `git log`, refreshed
  in the browser from GitHub's public API. A shallow clone limits how far back
  it reaches; build from a full clone.

Each route's own prose lives in a `content.md` beside its `page.tsx`, divided
into named `<!-- @block -->` sections (`src/lib/page-content.ts`). A missing
block fails the build rather than rendering an empty section.

## Tests

- `npm test` runs the unit tests (vitest): the palette's contrast ratios, the
  link rewriting, the scene reader, the activity feed's parsing.
- `npm run test:e2e` runs the browser tests (Playwright) against `out/`, so
  build first. A machine with its own Chromium can use it instead of the
  build Playwright pins: set `PLAYWRIGHT_CHROMIUM_EXECUTABLE` to its path.
- `npm run test:play` runs `/play`'s specs alone (`tests/e2e/play*.spec.ts`;
  the smoke test is `play.spec.ts`): a library scene compiles and draws, and
  a shader error is marked on its line. It asks Chromium for SwiftShader, so
  it needs no GPU. `npm run browser` installs the headless Chromium it runs
  in.

## In CI

CI's `site` job (D552) runs `cargo xtask ci --strict site` from the
repository root on every pull request that touches the site, the scenes or
a crate the engine is built from: `npm ci`, lint, typecheck, the unit
tests, `npm run build` (the engine included, built `--locked` and bound by
`wasm-bindgen` at Cargo.lock's version) and `npm run test:play`. The rest of the
Playwright suite runs by hand with `npm run test:e2e`. The same
lane runs on a desk with `cargo xtask ci site`. Node is the release in
`.nvmrc`.

## Deploying

The site runs on Cloudflare as a Worker at
<https://strobe.lnorton89.workers.dev> (D525): the export in `out/` served
as static files, plus a small script in `worker/` for `/play`'s short share
links (D527). `wrangler.jsonc` describes it.

```bash
npm run deploy:cf
```

builds the site for that address and deploys it as the account in
`~/.config/strobe/cloudflare.env` (`CLOUDFLARE_ACCOUNT_ID`,
`CLOUDFLARE_API_TOKEN` with Workers Scripts and Workers R2 Storage Edit),
never the machine's wrangler login. `npm run deploy:cf:preview` uploads a
version with its own preview URL instead.

The export alone still works on any static host (`netlify.toml` describes a
Netlify deploy, `npm run deploy`); there, Share falls back to links that
carry the whole scene.

### Shared scenes

Share stores the scene as one JSON file in the R2 bucket `strobe-scenes`,
named by its content's hash (`scenes/<id>.json`), and links to `/s/<id>`.
Nothing is ever changed in place. To take down a shared scene someone
reported, delete its object; the link then says there is no such scene:

```bash
npx wrangler@4.156.0 r2 object delete strobe-scenes/scenes/<id>.json --remote
```

(Run it with the same credentials as the deploy, for example by setting
`CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` in that shell only.)

## Credits

The site's structure, components and test suite are adapted from the
[Cinnabar site](https://github.com/bonzupii/cinnabar/tree/main/site),
Copyright 2026 Micah Ostrow, used under the Apache License 2.0
([`LICENSE-APACHE`](LICENSE-APACHE)). It has been modified: re-themed to the
Strobe brand, with Strobe's own pages, content, icons, mark and social cards,
and the Cinnabar-specific features (the playground, its compiler and syntax
highlighter, the learning pages) removed.

The social cards are rendered at build time by
[Metaplate](https://github.com/freeknet/metaplate) (MIT), with Satori and resvg-js
(both MPL-2.0). None of the three ships to a reader.

The site's fonts are self-hosted from the `@fontsource` npm packages, so a build
asks no font service for anything: [Space Grotesk](https://github.com/floriankarsten/space-grotesk)
(Copyright 2020 The Space Grotesk Project Authors),
[Inter](https://github.com/rsms/inter) (Copyright 2016 The Inter Project Authors)
and [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) (Copyright 2020
The JetBrains Mono Project Authors), all under the SIL Open Font License 1.1.
`scripts/sync-assets.mjs` publishes each licence beside the export, at
`/generated/fonts/`.

Strobe itself is MIT with the Commons Clause, so source-available rather than
open source; the repository's [`LICENSE`](../LICENSE) has the exact terms.
