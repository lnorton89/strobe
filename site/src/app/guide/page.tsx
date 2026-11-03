import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { GuideIcon } from "@/components/brand/icons";
import { ogImageMetadata } from "@/lib/og-image";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "User guide",
  title: "Running a show with Strobe.",
  description:
    "Install, first run, scenes, audio, the output, playlists, MIDI and OSC, remote control from a phone, Shadertoy import and what to do when something goes wrong.",
  alt: "Strobe social card — the user guide.",
};

export const metadata: Metadata = {
  title: "User guide",
  description:
    "How to run Strobe: install, the control and output windows, scenes and controls, audio, playlists, MIDI and OSC, phone remotes, venue mode and Shadertoy import.",
  alternates: { canonical: "/guide/" },
  ...ogImageMetadata("/guide/", og),
};

export default async function GuidePage() {
  const markdown = await readRepoDoc("docs/USER_GUIDE.md");
  return (
    <RepoDocPage
      section="Guide"
      note="For running Strobe, not building it"
      icon={GuideIcon}
      title="Running a show with Strobe."
      lede="Everything a performer needs, from unzipping the release to a venue machine that runs a show unattended. You do not need Rust or a compiler for any of it."
      source={linkRepoFile(
        "This page is `docs/USER_GUIDE.md`, rendered at build time. Fixes to it are fixes to that file.",
        "docs/USER_GUIDE.md",
      )}
      markdown={markdown}
      tocLabel="Sections"
    />
  );
}
