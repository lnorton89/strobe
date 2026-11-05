/*
 * The social cards' copy, one card per exported route.
 *
 * Plain data with no imports from Next or the renderer, because two different
 * programs read it: the pages, for their metadata, and `metaplate render`,
 * through ./cards.render.tsx. The `Record` over the route list makes a route
 * without a card a type error.
 */

import type { ExportedRoute } from "../site";

export type CardCopy = {
  /** Small uppercase label above the title. */
  eyebrow: string;
  title: string;
  /** One short sentence. The guide says to avoid feature lists and tiny copy. */
  description: string;
  /** Alternative text for the rendered card. */
  alt: string;
  /** The library scene whose frame fills the card's right side (guide section 7). */
  scene: string;
};

export const CARDS: Record<ExportedRoute, CardCopy> = {
  "/": {
    eyebrow: "VJ engine",
    title: "A VJ engine for audio-reactive GLSL scenes.",
    description: "Plays Synesthesia scenes unchanged, imports Shadertoy shaders, runs playlists.",
    alt: "Strobe, a VJ engine for audio-reactive GLSL scenes, beside a frame of its Electric Bloom scene.",
    scene: "14-electric-bloom",
  },
  "/download/": {
    eyebrow: "Download",
    title: "Unzip it and run it.",
    description: "A Windows release with no installer. Linux from source, with a Nix flake.",
    alt: "Strobe's download page, beside a frame of its Chromatic Rise scene.",
    scene: "13-chromatic-rise",
  },
  "/guide/": {
    eyebrow: "User guide",
    title: "Running a show with Strobe.",
    description: "Scenes, audio, playlists, MIDI and OSC, phone remotes and Shadertoy import.",
    alt: "Strobe's user guide, beside a frame of its Neon Ribbon Flow scene.",
    scene: "15-neon-ribbon-flow",
  },
  "/scenes/": {
    eyebrow: "Scene library",
    title: "The scene library.",
    description: "Original scenes, from a drifting gradient to raymarched clouds.",
    alt: "Strobe's scene library, beside a frame of its Voronoi Glass scene.",
    scene: "04-voronoi-glass",
  },
  "/play/": {
    eyebrow: "Play",
    title: "Write a Strobe scene in your browser.",
    description: "Strobe's own scene player, compiled to WebAssembly and running on WebGL2.",
    alt: "Strobe's browser sandbox, beside a frame of its Plasma Pulse scene.",
    scene: "02-plasma-pulse",
  },
  "/compatibility/": {
    eyebrow: "Compatibility",
    title: "What works, where, and the evidence.",
    description: "Supported, degraded, unsupported or unverified, each with what shows it.",
    alt: "Strobe's compatibility matrix, beside a frame of its SDF Cathedral scene.",
    scene: "09-sdf-cathedral",
  },
  "/architecture/": {
    eyebrow: "Architecture",
    title: "How Strobe is put together.",
    description: "The crate graph, the frame loop, and the one crate that touches the GPU.",
    alt: "Strobe's architecture, beside a frame of its Silicon Atlas scene.",
    scene: "12-silicon-atlas",
  },
  "/roadmap/": {
    eyebrow: "Roadmap",
    title: "What is built, and where it was proved.",
    description: "What is implemented, on which machine it was checked, and what comes next.",
    alt: "Strobe's roadmap, beside a frame of its Aurora Gradient scene.",
    scene: "01-aurora-gradient",
  },
};
