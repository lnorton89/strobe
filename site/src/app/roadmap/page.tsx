import type { Metadata } from "next";
import ActivityFeed from "@/components/ActivityFeed";
import Disclosure from "@/components/Disclosure";
import DocBody from "@/components/DocBody";
import PageHeader from "@/components/PageHeader";
import Reveal from "@/components/Reveal";
import SectionHeading from "@/components/SectionHeading";
import { ArrowLink, SourceNote } from "@/components/ui";
import { GitHubIcon, GuideIcon, RoadmapIcon } from "@/components/brand/icons";
import { CONTAINER } from "@/lib/constants";
import { COMMITS_URL, fetchCommits } from "@/lib/github";
import { isShallow, readGitLog } from "@/lib/git-log";
import { ogImageMetadata } from "@/lib/og-image";
import { readPageContent } from "@/lib/page-content";
import { linkRepoFile, readRepoDoc, splitRoadmap } from "@/lib/repo-docs";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Roadmap",
  title: "What is built, and where it was proved.",
  description:
    "The roadmap and status of Strobe: what is implemented and integrated, what is in flight, the priorities next, and the known gaps.",
  alt: "Strobe social card — the roadmap.",
};

export const metadata: Metadata = {
  title: "Roadmap",
  description:
    "Strobe's roadmap: what is implemented and on which machine it was verified, what is in flight, what comes next, and the known gaps — with the repository's recent activity.",
  alternates: { canonical: "/roadmap/" },
  ...ogImageMetadata("/roadmap/", og),
};

export default async function RoadmapPage() {
  const [document, content, commits] = await Promise.all([
    readRepoDoc("docs/ROADMAP.md"),
    readPageContent("roadmap"),
    /*
     * The build's copy of the commit feed, prerendered into the HTML so the
     * section is correct without JavaScript and when GitHub cannot be reached.
     * `fetchCommits` never rejects, so a build behind a firewall or with the
     * API rate-limited produces the section's static fallback rather than
     * failing the deploy. The timeout is what stops a hung request hanging it.
     */
    fetchCommits({ timeoutMs: 5000, revalidateSeconds: 900 }),
  ]);
  const { body, updated } = splitRoadmap(document);

  /*
   * The history the section's windows are measured against, read from the
   * repository this site is built inside rather than from the API, which
   * pages at a hundred commits a request.
   */
  const history = readGitLog();

  if (commits.length === 0) {
    console.warn(
      "[roadmap] GitHub returned no commits at build time; the activity feed" +
        " will ship the repository's own log and fill in from the browser.",
    );
  }
  if (history.length === 0) {
    console.warn(
      "[roadmap] git returned no commits at build time; the activity feed's" +
        " windows will cover only the commits the API returned.",
    );
  } else if (isShallow()) {
    console.warn(
      "[roadmap] the checkout is shallow, so the activity feed's history" +
        " reaches only as far back as the clone depth.",
    );
  }

  return (
    <article className="pb-28">
      <PageHeader
        section="Roadmap"
        note={updated ? `docs/ROADMAP.md · updated ${updated}` : "docs/ROADMAP.md"}
        icon={RoadmapIcon}
        title="What is built, and where it was proved."
        lede={content.block("lede")}
      />

      <section className={`${CONTAINER} pt-16`}>
        <SectionHeading
          title="Recent activity"
          note={content.block("activity-note")}
          icon={GitHubIcon}
        />
        <Reveal className="mt-9">
          <ActivityFeed
            history={history}
            initial={commits}
            fallback={content.block("activity-fallback")}
          />
          <div className="mt-6">
            <ArrowLink href={COMMITS_URL} external>
              The full commit log
            </ArrowLink>
          </div>
        </Reveal>
      </section>

      <section className={`${CONTAINER} pt-24 pb-12`}>
        <SectionHeading title="The full record" note="docs/ROADMAP.md" icon={GuideIcon} />
        <SourceNote className="mt-10">
          {linkRepoFile(
            linkRepoFile(content.block("source"), "docs/ROADMAP.md"),
            "docs/STATUS_AUDIT.md",
          )}
        </SourceNote>
      </section>

      {/*
        The document itself, open: on this page it is the content. Links into
        it from the other pages land on its anchors.
      */}
      <div className={CONTAINER}>
        <Disclosure summary={content.block("document")} defaultOpen>
          <DocBody markdown={body} tocLabel="Roadmap" />
        </Disclosure>
      </div>
    </article>
  );
}
