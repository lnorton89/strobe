import type { Metadata } from "next";
import PlayApp from "@/components/play/PlayApp";
import { ogImageMetadata } from "@/lib/og-image";
import { readScenes } from "@/lib/scenes";

/*
 * The shader sandbox (D524). The page is one client component; this server
 * half only gives it the scene library to open, read at build time like the
 * scenes page, and the metadata.
 */

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Play",
  title: "Write a Strobe scene in your browser.",
  description:
    "Strobe's own scene player, compiled to WebAssembly: edit main.glsl, scene.json and script.js, play with the controls, feed it your microphone, and share the result as a link.",
  alt: "Strobe social card — the shader sandbox.",
};

export const metadata: Metadata = {
  title: "Play",
  description:
    "A sandbox for Strobe scenes: edit a Synesthesia-format scene's shader, controls and script and watch Strobe's own renderer play it, audio-reactive, in the browser.",
  alternates: { canonical: "/play/" },
  ...ogImageMetadata("/play/", og),
};

export default async function PlayPage() {
  const scenes = await readScenes();
  return (
    <>
      <h1 className="sr-only">Strobe scene sandbox</h1>
      <PlayApp library={scenes.map(({ slug, title }) => ({ slug, title }))} />
    </>
  );
}
