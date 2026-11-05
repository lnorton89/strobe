import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { GuideIcon } from "@/components/brand/icons";
import { pageMetadata } from "@/lib/og/metadata";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";

export const metadata: Metadata = pageMetadata("/guide/", {
  title: "User guide",
  description:
    "How to run Strobe: install, the control and output windows, scenes and controls, audio, playlists, MIDI and OSC, phone remotes, venue mode and Shadertoy import.",
});

export default async function GuidePage() {
  const markdown = await readRepoDoc("docs/USER_GUIDE.md");
  return (
    <RepoDocPage
      section="Guide"
      note="docs/USER_GUIDE.md"
      icon={GuideIcon}
      title="Running a show with Strobe."
      lede="From unzipping the release to a venue machine that runs a show unattended."
      source={linkRepoFile(
        "This page is `docs/USER_GUIDE.md`, rendered at build time. Fixes to it are fixes to that file.",
        "docs/USER_GUIDE.md",
      )}
      markdown={markdown}
      tocLabel="Sections"
    />
  );
}
