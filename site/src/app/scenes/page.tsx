import type { Metadata } from "next";
import PageHeader from "@/components/PageHeader";
import Reveal from "@/components/Reveal";
import SceneCard from "@/components/SceneCard";
import SectionHeading from "@/components/SectionHeading";
import ShellBlock from "@/components/ShellBlock";
import { GridFill, Prose, SourceNote } from "@/components/ui";
import { ScenesIcon, ShellIcon } from "@/components/brand/icons";
import { CONTAINER } from "@/lib/constants";
import { ogImageMetadata } from "@/lib/og-image";
import { readScenes } from "@/lib/scenes";
import { DEFAULT_BRANCH, REPO_URL } from "@/lib/site";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Scene library",
  title: "Original scenes, every one a real frame.",
  description:
    "Five tiers of GPU load from a drifting gradient to raymarched clouds, a scripted physics scene, and feedback-driven paint studies. All original work, MIT licensed.",
  alt: "Strobe social card — the scene library.",
};

export const metadata: Metadata = {
  title: "Scenes",
  description:
    "The original scenes that ship with Strobe: benchmark tiers from a gradient to volumetric clouds, a script.js physics scene, and audio-reactive feedback studies.",
  alternates: { canonical: "/scenes/" },
  ...ogImageMetadata("/scenes/", og),
};

export default async function ScenesPage() {
  const scenes = await readScenes();
  const tiered = scenes.filter((scene) => scene.tier !== undefined);
  const tiers = new Set(tiered.map((scene) => scene.tier));

  return (
    <article className="pb-28">
      <PageHeader
        section="Scenes"
        note={`${scenes.length} scenes · scenes/ in the repository`}
        icon={ScenesIcon}
        title="Original scenes, every one a real frame."
        lede={`Strobe ships with ${scenes.length} original scenes. ${tiered.length} of them span ${tiers.size} benchmark tiers of rising GPU load — the set \`strobe-bench\` times on every machine — and the rest are scripted and audio-reactive studies. Each thumbnail is the scene's own.`}
      />

      <section className={`${CONTAINER} pt-14`}>
        <div className="rule-grid grid sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {scenes.map((scene, index) => (
            <Reveal key={scene.slug} delay={Math.min((index % 4) * 0.03, 0.12)} className="flex">
              <SceneCard scene={scene} detailed />
            </Reveal>
          ))}
          <GridFill count={scenes.length} columns={{ base: 1, sm: 2, lg: 3, xl: 4 }} />
        </div>
      </section>

      <section className={`${CONTAINER} pt-24`}>
        <SectionHeading title="A scene is a folder" note=".synScene" icon={ShellIcon} />
        <div className="mt-9 grid gap-10 *:min-w-0 lg:grid-cols-2 lg:items-start">
          <Reveal>
            <Prose>
              {
                "Every scene is a Synesthesia scene folder: `main.glsl` for the shader, `scene.json` for its controls, passes and tags, and an optional `script.js`. Strobe reads them unchanged, and its own extensions are extra keys Synesthesia ignores.\n\nTo render every scene headless and get a frame, a timing and a contact sheet for each, run the bench."
              }
            </Prose>
          </Reveal>
          <Reveal delay={0.05}>
            <ShellBlock
              lines={[
                "cargo run --release -p strobe-bench -- --size 1280x720",
                { out: "target/bench/contact-sheet.png  results.md  <scene>.png" },
              ]}
              title="Render the library"
            />
          </Reveal>
        </div>
        <SourceNote className="mt-14">
          {`The scenes in \`scenes/\` are original work under the project's MIT licence. Timings for each tier on llvmpipe, the T480 and the GTX 1080 Ti are in [docs/BENCHMARKS.md](${REPO_URL}/blob/${DEFAULT_BRANCH}/docs/BENCHMARKS.md).`}
        </SourceNote>
      </section>
    </article>
  );
}
