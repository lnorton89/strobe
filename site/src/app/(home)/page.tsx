import Wordmark from "@/components/brand/Wordmark";
import DataTable from "@/components/DataTable";
import { InlineMarkdown } from "@/components/Markdown";
import { Eyebrow } from "@/components/PageHeader";
import Reveal from "@/components/Reveal";
import SceneCard from "@/components/SceneCard";
import Screenshot from "@/components/Screenshot";
import SectionHeading from "@/components/SectionHeading";
import { Action, ArrowLink, Stat } from "@/components/ui";
import {
  AudioIcon,
  CompatibilityIcon,
  ControllerIcon,
  DownloadIcon,
  GitHubIcon,
  GuideIcon,
  ImportIcon,
  PerformIcon,
  PlaylistIcon,
  RemoteIcon,
  ResilienceIcon,
  ScenesIcon,
} from "@/components/brand/icons";
import { ICON } from "@/lib/constants";
import { readPageContent } from "@/lib/page-content";
import { readScenes } from "@/lib/scenes";
import { BADGES, REPO_URL, STATUS_BADGE } from "@/lib/site";

export const og = {
  eyebrow: "Open-source VJ engine",
  title: "Audio-reactive shaders for screens, projectors and LED walls.",
  description:
    "Plays Synesthesia scenes as they are, imports Shadertoy shaders and runs long playlists — built to stay up on old laptops.",
  alt: "Strobe — audio-reactive shaders for screens, projectors and LED walls.",
};

/**
 * The icon each feature is drawn with, bound to its `###` heading in
 * content.md by slug. Rewording a heading without updating this fails the
 * build, naming the slug.
 */
const FEATURE_ICONS = {
  "plays-synesthesia-scenes": AudioIcon,
  "imports-shadertoy": ImportIcon,
  "generates-controls": ControllerIcon,
  "runs-long-shows": PlaylistIcon,
  "takes-show-control": ControllerIcon,
  "remote-from-a-phone": RemoteIcon,
  "a-performer-first-ui": PerformIcon,
  "stays-up-on-bad-hardware": ResilienceIcon,
} as const;

/** README.md's comparison table, row for row. */
const COMPARISON = [
  [
    "Scene format",
    "SSF",
    "SSF, read as-is; Strobe's extensions are extra keys Synesthesia ignores",
  ],
  ["Shadertoy", "Ported by hand", "Imported from the page, multipass, live audio"],
  [
    "Playlists",
    "Each item carries its own settings",
    "Inherited settings, a tag-based generator, beat-locked lengths, a wall-clock anchor",
  ],
  ["Broken GL driver", "Does not start", "Falls back to ANGLE"],
  [
    "A while(true){} in a script",
    "Hangs",
    "Interrupted; the scene keeps rendering",
  ],
] as const;

/** How many scenes the home page previews; the rest are one click away. */
const SCENE_PREVIEW = 8;

export default async function Home() {
  const [content, scenes] = await Promise.all([readPageContent("(home)"), readScenes()]);

  return (
    <>
      <section className="hero-field relative overflow-hidden">
        <div className="mx-auto max-w-350 px-6 pt-16 pb-20 sm:px-10 sm:pt-24 sm:pb-28">
          <div className="grid gap-14 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] lg:items-center lg:gap-16">
            <div className="flex flex-col gap-7">
              <Eyebrow>Open-source VJ engine</Eyebrow>
              <h1>
                <Wordmark size="clamp(40px, 5.4vw, 72px)" withMark withDescriptor />
              </h1>
              <p className="text-text font-display text-[28px] leading-[1.12] font-semibold tracking-[-0.02em] text-balance sm:text-[38px]">
                {content.block("tagline")}
              </p>
              <div className="text-secondary text-[17px] leading-[1.65] text-pretty [&_code]:font-mono [&_strong]:text-text">
                <InlineMarkdown>{content.block("hero-why")}</InlineMarkdown>
              </div>
              <div className="rule-grid flex w-fit flex-wrap">
                {BADGES.map((badge) => (
                  <span
                    key={badge}
                    className="bg-ground text-secondary grow px-4 py-2.5 text-center font-mono text-xs"
                  >
                    {badge}
                  </span>
                ))}
                <span className="bg-accent text-on-accent grow px-4 py-2.5 text-center font-mono text-xs font-medium">
                  {STATUS_BADGE}
                </span>
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-4">
                <Action href="/download/" variant="primary" icon={DownloadIcon}>
                  Download
                </Action>
                <Action href="/guide/" icon={GuideIcon}>
                  User guide
                </Action>
                <Action href={REPO_URL} variant="ghost" icon={GitHubIcon} external>
                  Source
                </Action>
              </div>
            </div>
            <div className="min-w-0">
              <Screenshot
                src="/generated/docs-images/app-control.png"
                alt="Strobe's control window playing Script Orbits: the live preview at the centre, the scene's controls in the side panel, and the transport, macros and BLACKOUT along the bottom."
                width={1440}
                height={900}
                title="Perform"
                priority
              />
              <p className="text-secondary mt-4 text-[13px] leading-[1.65]">
                {content.block("hero-proof")}
              </p>
            </div>
          </div>
        </div>
      </section>

      <section className="border-hairline bg-panel border-y">
        <div className="mx-auto max-w-350 px-6 py-24 sm:px-10">
          <SectionHeading
            title="What it does"
            note={content.block("features-note")}
            icon={PerformIcon}
          />
          <div className="rule-grid mt-11 grid sm:grid-cols-2 xl:grid-cols-4">
            {content.items("features").map(({ slug, title, body }, index) => {
              const Icon = FEATURE_ICONS[slug as keyof typeof FEATURE_ICONS];
              if (!Icon) throw new Error(`No icon bound for feature "${slug}"`);
              return (
                <Reveal
                  key={slug}
                  delay={Math.min(index * 0.03, 0.15)}
                  className="bg-ground hover:bg-panel-raised panel-hover flex flex-col gap-4 p-7"
                >
                  <Icon size={ICON.card} className="text-text" />
                  <h3 className="text-text text-[19px] leading-snug font-semibold tracking-[-0.015em]">
                    {title}
                  </h3>
                  <div className="text-secondary text-[14.5px] leading-[1.65] text-pretty [&_code]:font-mono [&_code]:text-[0.9em]">
                    <InlineMarkdown>{body}</InlineMarkdown>
                  </div>
                </Reveal>
              );
            })}
          </div>
        </div>
      </section>

      <section className="border-hairline border-b">
        <div className="mx-auto max-w-350 px-6 py-24 sm:px-10">
          <SectionHeading
            title="The scene library"
            note={content.block("scenes-note")}
            icon={ScenesIcon}
          />
          <Reveal className="mt-9 flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
            <p className="text-secondary max-w-[72ch] text-[16.5px] leading-[1.7] text-pretty">
              {content.block("scenes")}
            </p>
            <ArrowLink href="/scenes/" className="flex-none">
              All {scenes.length} scenes
            </ArrowLink>
          </Reveal>
          <div className="rule-grid mt-10 grid grid-cols-2 lg:grid-cols-4">
            {scenes.slice(0, SCENE_PREVIEW).map((scene) => (
              <SceneCard key={scene.slug} scene={scene} />
            ))}
          </div>
        </div>
      </section>

      <section className="mx-auto max-w-350 px-6 py-24 sm:px-10">
        <SectionHeading
          title="Measured, not claimed"
          note={content.block("numbers-note")}
          icon={CompatibilityIcon}
        />
        <Reveal className="rule-grid mt-11 grid grid-cols-2 lg:grid-cols-4">
          <Stat value="63 / 65" label="Owner scenes play with scripts, NVIDIA" />
          <Stat value="87 / 90" label="Corpus scenes render, both GL dialects" />
          <Stat value="GL 3.3" label="The floor: a 2012 Intel HD 4000" />
          <Stat value="4" label="Windows machines on record: Intel, NVIDIA, AMD" />
        </Reveal>
        <Reveal className="mt-20">
          <SectionHeading title="Strobe and Synesthesia" note={content.block("compare-note")} />
          <DataTable
            headings={["Area", "Synesthesia", "Strobe"]}
            data={COMPARISON}
            className="mt-9"
          />
        </Reveal>
      </section>

      <section className="border-hairline bg-panel border-t">
        <Reveal className="mx-auto flex max-w-350 flex-col gap-6 px-6 py-20 sm:px-10 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <Eyebrow>Project status</Eyebrow>
            <h2 className="text-text mt-4 text-[28px] font-semibold tracking-tight">
              {content.block("status-title")}
            </h2>
            <p className="text-secondary mt-3 max-w-[70ch] leading-[1.7]">
              {content.block("status")}
            </p>
          </div>
          <div className="flex flex-wrap gap-4">
            <Action href="/compatibility/" variant="primary" icon={CompatibilityIcon}>
              Compatibility
            </Action>
            <Action href="/roadmap/">Roadmap</Action>
          </div>
        </Reveal>
      </section>
    </>
  );
}
