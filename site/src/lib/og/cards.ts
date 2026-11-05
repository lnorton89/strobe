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
    eyebrow: "Open-source VJ engine",
    title: "Audio-reactive shaders for screens, projectors and LED walls.",
    description:
      "Plays Synesthesia scenes as they are, imports Shadertoy shaders and runs long playlists — built to stay up on old laptops.",
    alt: "Strobe — audio-reactive shaders for screens, projectors and LED walls.",
    scene: "14-electric-bloom",
  },
  "/download/": {
    eyebrow: "Download",
    title: "Unzip it and run it.",
    description:
      "A Windows release with no installer and an ANGLE fallback beside it; Linux from source with a package list and a Nix flake.",
    alt: "Strobe social card — download and build.",
    scene: "13-chromatic-rise",
  },
  "/guide/": {
    eyebrow: "User guide",
    title: "Running a show with Strobe.",
    description:
      "Install, first run, scenes, audio, the output, playlists, MIDI and OSC, remote control from a phone, Shadertoy import and what to do when something goes wrong.",
    alt: "Strobe social card — the user guide.",
    scene: "15-neon-ribbon-flow",
  },
  "/scenes/": {
    eyebrow: "Scene library",
    title: "Original scenes, every one a real frame.",
    description:
      "Five tiers of GPU load from a drifting gradient to raymarched clouds, a scripted physics scene, and feedback-driven paint studies. All original work, MIT licensed.",
    alt: "Strobe social card — the scene library.",
    scene: "04-voronoi-glass",
  },
  "/play/": {
    eyebrow: "Play",
    title: "Write a Strobe scene in your browser.",
    description:
      "Strobe's own scene player, compiled to WebAssembly: edit main.glsl, scene.json and script.js, play with the controls, feed it your microphone, and share the result as a link.",
    alt: "Strobe social card — the shader sandbox.",
    scene: "02-plasma-pulse",
  },
  "/compatibility/": {
    eyebrow: "Compatibility",
    title: "What works, where, and the evidence.",
    description:
      "Supported, degraded, unsupported and unverified: the scene format, scripts, real Synesthesia scenes, Shadertoy, graphics backends, audio, playlists and show control.",
    alt: "Strobe social card — the compatibility matrix.",
    scene: "09-sdf-cathedral",
  },
  "/architecture/": {
    eyebrow: "Architecture",
    title: "Fifteen crates, one GPU boundary.",
    description:
      "Only the app knows about everything, only strobe-gfx touches the GPU, and everything that can be tested without a GPU is.",
    alt: "Strobe social card — the architecture.",
    scene: "12-silicon-atlas",
  },
  "/roadmap/": {
    eyebrow: "Roadmap",
    title: "What is built, and where it was proved.",
    description:
      "The roadmap and status of Strobe: what is implemented and integrated, what is in flight, the priorities next, and the known gaps.",
    alt: "Strobe social card — the roadmap.",
    scene: "01-aurora-gradient",
  },
};
