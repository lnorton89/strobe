# STROBE Brand Identity Guidelines

**Direction:** Concept 03 — Parametric Flow  
**Version:** 1.0 · Working brand direction

## Brand at a glance

**STROBE** is an open-source, real-time shader visualizer and VJ engine. Its identity should feel like a precise creative instrument: technical enough for shader authors and performers, expressive enough to represent generative visuals, and restrained enough to remain credible in developer tools and documentation.

- **Core idea:** Continuous geometry in motion.
- **Attributes:** Expressive, technical, open, precise, experimental, performance-ready.
- **Descriptor:** Shader visualizer.
- **Supporting language:** Real-time shaders · Generative graphics · Open source.

**Positioning statement:** STROBE turns real-time signals and shader code into expressive visual systems for screens, projectors, and live environments.

## 1. Creative direction

Concept 03 uses a continuous parametric ribbon: a loop that appears to fold through itself, with fine parallel contours revealing its underlying geometry. It communicates motion, phase, recursion, and a form that is generated rather than illustrated.

The mark should feel like a rendered mathematical object—not a music festival logo, gaming badge, or generic “AI” symbol. Keep the silhouette memorable and detail controlled.

### Design principles

- **Geometry first:** The silhouette must read before contour detail.
- **Color with purpose:** Cyan-to-violet-to-magenta indicates motion and depth; it is not required for every use.
- **Quiet typography:** Let the mark and the software output carry the expressive weight.
- **Open-source credibility:** Use familiar, legible layouts and avoid unnecessary gloss.
- **Real output over mockups:** Product imagery should come from actual STROBE scenes and the running application.

## 2. Logo system

- **Primary lockup:** Full-color parametric mark above the STROBE wordmark, with “SHADER VISUALIZER” below. Use on dark backgrounds for README headers, splash screens, and brand presentations.
- **Horizontal lockup:** Mark on the left, wordmark and descriptor on the right. Use in narrow banners, website headers, release graphics, and documentation footers.
- **Symbol-only mark:** Ribbon alone for application icons, taskbar icons, favicons, and compact UI placements.
- **Monochrome mark:** Single-color version for print, low-color contexts, and backgrounds where the gradient lacks sufficient contrast.

### Clear space and minimum size

Preserve clear space of at least the height of the wordmark’s “S” around the full lockup. For the symbol alone, keep clear space equal to one quarter of the symbol’s width.

| Asset | Starting target |
|---|---|
| Simplified symbol | 16 px minimum; 24 px preferred |
| Full-color symbol with contours | 32 px preferred |
| Horizontal lockup | 180 px wide minimum |
| Stacked lockup | 140 px wide minimum |

At small sizes, remove fine contour lines and use a simplified solid ribbon silhouette. Do not merely shrink the detailed master.

### Do not

- Stretch, skew, or arbitrarily rotate the mark.
- Bake a glow into the canonical logo file.
- Apply the gradient to every letter.
- Place the mark on noisy imagery without a quiet field.
- Redraw the ribbon inconsistently or add unapproved outlines and shadows.

## 3. Color palette

The palette is designed for dark application chrome, with the mark’s gradient as a controlled expressive accent.

| Token | Hex | Role |
|---|---|---|
| Void | `#080B14` | Main background |
| Panel | `#101725` | Raised surfaces |
| Border | `#253247` | Dividers and subtle outlines |
| Text | `#F3F6FA` | Primary text |
| Muted | `#9AA8BC` | Secondary text |
| Cyan | `#00E5FF` | Primary accent / active state |
| Blue | `#3382F6` | Secondary accent |
| Violet | `#8A55F7` | Depth / transition |
| Magenta | `#EC4B99` | Expressive highlight |
| Slate | `#94A3B8` | Neutral UI accent |

**Primary gradient:** Cyan → Blue → Violet → Magenta. Suggested stops: 0%, 35%, 68%, 100%. Use for the mark, selected visual highlights, and occasional promotional graphics—not for long text or large areas of application chrome.

**Balance:** Roughly 70% dark neutrals, 20% light/neutral text and surfaces, and 10% accents. This is a visual guide, not a rigid measurement.

**Accessibility:** Body text should meet WCAG AA contrast against its background. Do not use cyan, violet, or magenta as small text on Void without checking contrast. Use Text or Muted for readable copy; reserve saturated colors for icons, focus indicators, charts, and short labels.

## 4. Typography

- **Primary:** Space Grotesk for headings and wordmark treatment where available.
- **Secondary:** Inter for body copy, controls, tables, labels, and documentation.
- **Code:** JetBrains Mono or the platform monospace for shader code and technical values.
- **Fallbacks:** Use system sans-serif fonts if the preferred fonts cannot be bundled.

| Style | Size / leading | Weight and use |
|---|---|---|
| Display | 48–72 px | Medium; spaced wordmark treatment |
| H1 | 32 / 1.15 | Semibold |
| H2 | 24 / 1.25 | Semibold |
| H3 | 18 / 1.35 | Medium |
| Body | 14–16 / 1.5 | Regular |
| UI labels | 11–12 / 1.4 | Medium; restrained tracking |
| Code | 12–14 px | Monospace |

Use wide letter spacing only for the wordmark and short all-caps labels, never paragraphs.

## 5. Graphic language

- **Parametric contours:** Fine parallel lines following the ribbon’s curvature. Use as a supporting motif, not as a substitute for the actual mark.
- **Flow fields:** Broad, low-contrast line fields can frame README or release artwork. Keep the central content clear.
- **Spectral fields:** Prefer real shader output with cyan, blue, violet, and magenta accents. Favor rich forms and controlled contrast over random neon noise.
- **Grid and calibration details:** Occasional fine rules, coordinate ticks, or small technical labels add an instrument-like character. Keep them subtle and functional.
- **Motion:** If animated, the ribbon should deform or rotate slowly and continuously. Avoid rapid flashes or strobing the logo itself. Respect reduced-motion preferences and provide a static fallback.

## 6. Product UI principles

The application is the instrument; the shader output is the main event.

- Use a near-black canvas and restrained panel surfaces.
- Keep scene preview/output visually dominant.
- Make controls clear, compact, and predictable.
- Use cyan for active selection and focus, not for every clickable element.
- Use violet and magenta mainly in visual output and selected highlights.
- Separate navigation, scene selection, parameters, and playback/transport controls.
- Show readable values, units, and slider endpoints where useful.
- Keep borders subtle; avoid excessive blur, glow, gradients, and nested cards.
- Use the same symbol in the app window, taskbar, installer, and documentation.

## 7. Image and screenshot direction

**Prefer:** Authentic application screenshots, real shader renders, scene contact sheets, parameter controls, and fullscreen output captured from STROBE.

**Shader imagery:** Use compositions with a clear focal form, layered depth, and a limited palette. Include variety across fluid, wireframe, particle, spectral, and geometric scenes without implying that a scene is built in unless it actually is.

**Screenshot standards:** Capture at useful resolution; remove unrelated desktop clutter; use consistent window sizing; show the actual UI; ensure text is legible; label images by purpose rather than adding large decorative captions.

- **README banner:** Horizontal lockup on a dark field with a subtle flow-field texture and restrained spectral glow.
- **Social preview:** One strong real render, compact mark, and STROBE wordmark. Avoid feature lists and tiny copy.

## 8. Asset inventory and export standards

Keep a canonical vector source and generate raster derivatives from it. Avoid manually maintaining near-duplicate versions.

Suggested structure:

```text
assets/brand/
  mark.svg
  mark-mono.svg
  wordmark.svg
  lockup-horizontal.svg
  lockup-stacked.svg
  icon-16.png
  icon-32.png
  icon-64.png
  icon-128.png
  icon-256.png
  icon-512.png
  icon.ico
  banner.png
  social-preview.png
  README.md
assets/screenshots/
  app.png
  scenes.png
  output.png
docs/
  branding.md
```

**Generation rules:** Export transparent PNGs for marks, keep banner backgrounds opaque, use sRGB, and render each size from the vector master. For icons below 32 px, use a simplified geometry variant with fewer contour lines. Generate the Windows ICO from the approved icon set. Keep the SVG source and generation script in version control.

## 9. Voice and writing

**Voice:** Direct, curious, technically grounded, visually literate, open-source minded.

Write like a capable creative tool, not a lifestyle brand. Prefer specific descriptions of what the software does. Avoid inflated claims and hype.

**Use:** “Real-time shader visualization.” “Build and perform with generative visuals.” “Open-source VJ engine.” “Explore scenes, tune parameters, and render live.”

**Avoid:** “Unleash your creativity.” “The future of visual experiences.” “Revolutionary next-generation immersion.” Avoid claims of professional-grade maturity unless the software supports them.

## 10. Brand in use

- **Repository README:** Lead with the lockup, a short factual description, and a real screenshot or render. Keep decorative branding secondary to installation and project information.
- **Application icon:** Symbol-only, centered, with sufficient internal padding. Dark tile is the default; light and monochrome variants are optional.
- **Release graphics:** Use the primary gradient and a real shader frame. Version and release title must remain more readable than decoration.
- **Website/docs:** Use dark neutral surfaces, clear hierarchy, compact accent rules, and real app imagery. Avoid making every panel a gradient panel.

## 11. Implementation checklist

- [ ] Finalize the ribbon silhouette as a clean vector path.
- [ ] Approve full-color, monochrome, light-background, and simplified small-size variants.
- [ ] Test at 16, 24, 32, 64, 128, 256, and 512 px on dark and light surfaces.
- [x] Generate PNG and ICO assets from the canonical source. (`cargo xtask brand`, D190)
- [ ] Confirm app window, taskbar, installer, README, and release pages use the same mark.
- [ ] Capture actual application screenshots and real shader output.
- [x] Add a documented, repeatable asset-generation command. (`cargo xtask brand`, D190)
- [x] Add CI checks for required files and expected image dimensions. (`cargo xtask brand --check` compares every file byte for byte with a fresh render.)
- [ ] Check text contrast and keyboard focus states in the application UI.
- [ ] Include a static fallback for animated brand elements.

## Production note

This guide defines the intended identity and production rules. The generated concept board is visual direction, not final vector artwork. The parametric ribbon must be refined into reproducible vector geometry and tested at small sizes before shipping. Concept UI and renders must not be presented as actual application screenshots.
