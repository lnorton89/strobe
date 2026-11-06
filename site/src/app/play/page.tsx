import type { Metadata } from "next";
import { ArrowLink } from "@/components/Action";
import PlayApp from "@/components/play/PlayApp";
import { pageMetadata } from "@/lib/og/metadata";
import { readScenes } from "@/lib/scenes";
import { CONTAINER } from "@/lib/constants";

/*
 * The shader sandbox (D524). The page is one client component; this server
 * half only gives it the scene library to open, read at build time like the
 * scenes page, and the metadata.
 */

export const metadata: Metadata = pageMetadata("/play/", {
  title: "Play",
  description:
    "A sandbox for Strobe scenes: edit a Synesthesia-format scene's shader, controls and script and watch Strobe's own renderer play it, audio-reactive, in the browser.",
});

export default async function PlayPage() {
  const scenes = await readScenes();
  return (
    <>
      <h1 className="sr-only">Strobe scene sandbox</h1>
      <PlayApp library={scenes.map(({ slug, title }) => ({ slug, title }))} />
      <section className="border-hairline border-t">
        <div className={`${CONTAINER} py-14`}>
        <h2 className="text-text text-[22px] font-semibold tracking-tight">What this is</h2>
        <div className="text-secondary mt-4 grid max-w-[110ch] gap-x-12 gap-y-4 text-[15px] leading-[1.65] text-pretty md:grid-cols-2">
          <p>
            Strobe&apos;s own scene player, compiled to WebAssembly and drawing on WebGL2, so a
            scene and its errors behave as they do in the app. Pick a scene from Open, or edit
            the shader and it recompiles as you type. A synthetic 120 BPM track drives the audio
            until you choose your microphone or a file.
          </p>
          <p>
            Your scene saves itself in this browser, and nothing leaves it unless you press
            Share, which stores the scene and gives you a short link. Export writes a{" "}
            <code className="font-mono text-[0.9em]">.synScene</code> zip the app opens. Scripts
            run in a worker, with a shim of the app&apos;s JavaScript API.
          </p>
        </div>
        <div className="mt-8 flex flex-wrap gap-x-8 gap-y-3">
          <ArrowLink href="/download/">Download the app</ArrowLink>
          <ArrowLink href="/scenes/">The scene library</ArrowLink>
        </div>
        </div>
      </section>
    </>
  );
}
