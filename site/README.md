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

- **`/guide/`, `/compatibility/`, `/architecture/` and `/roadmap/`** render
  `docs/USER_GUIDE.md`, `docs/COMPATIBILITY.md`, `docs/ARCHITECTURE.md` and
  `docs/ROADMAP.md` (`src/lib/repo-docs.ts`). Links between those documents go
  to their pages here; every other repository link goes to GitHub.
- **`/scenes/`** is built from `scenes/*.synScene/scene.json`
  (`src/lib/scenes.ts`). A scene added to the repository appears on the next
  build.
- **Images** — the scene thumbnails, the screenshots in `docs/images/` and the
  icon PNGs from `assets/brand/` — are copied (thumbnails re-encoded to 640 px
  WebP) into the git-ignored `public/generated/` by `scripts/sync-assets.mjs`,
  which runs before `dev` and `build`.
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
highlighter, the learning pages) removed. Strobe itself is MIT licensed.
