import Wordmark from "@/components/brand/Wordmark";
import DataTable from "@/components/DataTable";
import { InlineMarkdown } from "@/components/Markdown";
import { Eyebrow } from "@/components/PageHeader";
import Reveal from "@/components/Reveal";
import SceneCard from "@/components/SceneCard";
import Screenshot from "@/components/Screenshot";
import SectionHeading from "@/components/SectionHeading";
import { Action, ArrowLink } from "@/components/Action";
import { CurrentReleaseSentence, ReleaseLink } from "@/components/Release";
import {
  CompatibilityIcon,
  DownloadIcon,
  GitHubIcon,
  PerformIcon,
  PlayIcon,
  ScenesIcon,
} from "@/components/brand/icons";
import { readPageContent } from "@/lib/page-content";
import { readScenes } from "@/lib/scenes";
import { BADGES, RELEASE_STAGE, REPO_URL } from "@/lib/site";

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

/** Prose under a heading in one of the plain grids. */
const CELL_BODY =
  "text-secondary text-[15px] leading-[1.65] text-pretty [&_code]:font-mono [&_code]:text-[0.9em]";

export default async function Home() {
  const [content, scenes] = await Promise.all([readPageContent("(home)"), readScenes()]);

  return (
    <>
      <section className="hero-field relative overflow-hidden">
        <div className="mx-auto max-w-350 px-6 pt-14 pb-16 sm:px-10 sm:pt-20 sm:pb-24">
          <div className="grid gap-12 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] lg:items-center lg:gap-16">
            <div className="flex flex-col gap-6">
              <h1>
                <Wordmark size="clamp(34px, 3.8vw, 52px)" withMark withDescriptor />
              </h1>
              <p className="text-text font-display text-[30px] leading-[1.12] font-semibold tracking-[-0.02em] text-balance sm:text-[42px]">
                {content.block("tagline")}
              </p>
              <div className="text-secondary text-[17px] leading-[1.65] text-pretty">
                <InlineMarkdown>{content.block("hero-why")}</InlineMarkdown>
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-4">
                <Action href="/download/" variant="primary" icon={DownloadIcon}>
                  Download
                </Action>
                <Action href="/play/" icon={PlayIcon}>
                  Try it in your browser
                </Action>
              </div>
              <p className="text-label flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-xs leading-relaxed">
                <ReleaseLink
                  suffix={RELEASE_STAGE}
                  className="text-accent-text hover:text-text underline decoration-1 underline-offset-[3px]"
                />
                {BADGES.map((badge) => (
                  <span key={badge}>{badge}</span>
                ))}
                <a
                  href={REPO_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="hover:text-text inline-flex items-center gap-1.5"
                >
                  <GitHubIcon size={13} />
                  Source
                </a>
              </p>
            </div>
            <div className="min-w-0">
              <Screenshot
                src="/generated/docs-images/app-control.png"
                alt="Strobe's control window playing Electric Bloom: the scene library in the middle, the live preview and the scene's controls in the side panel, and the transport, macros and BLACKOUT along the bottom."
                width={1440}
                height={900}
                title="Library"
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
        <div className="mx-auto max-w-350 px-6 py-20 sm:px-10 sm:py-24">
          <SectionHeading title="What it does" icon={PerformIcon} />
          <div className="rule-grid mt-10 grid md:grid-cols-2 xl:grid-cols-3">
            {content.items("features").map(({ slug, title, body }, index) => (
              <Reveal
                key={slug}
                delay={Math.min(index * 0.03, 0.15)}
                className="bg-ground flex flex-col gap-3 p-7"
              >
                <h3 className="text-text text-[19px] leading-snug font-semibold tracking-[-0.015em]">
                  {title}
                </h3>
                <div className={CELL_BODY}>
                  <InlineMarkdown>{body}</InlineMarkdown>
                </div>
              </Reveal>
            ))}
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
        <Reveal>
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
              <CurrentReleaseSentence />
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
