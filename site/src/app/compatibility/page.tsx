import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { CompatibilityIcon } from "@/components/brand/icons";
import { pageMetadata } from "@/lib/og/metadata";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";

export const metadata: Metadata = pageMetadata("/compatibility/", {
  title: "Compatibility",
  description:
    "Strobe's compatibility matrix: which SSF features, scripts, Synesthesia scenes, GPUs, drivers and controllers are supported, degraded or unverified, each with its evidence.",
});

export default async function CompatibilityPage() {
  const markdown = await readRepoDoc("docs/COMPATIBILITY.md");
  return (
    <RepoDocPage
      section="Compatibility"
      note="docs/COMPATIBILITY.md"
      icon={CompatibilityIcon}
      title="What works, where, and the evidence."
      lede="A type, a parser or a UI control existing is never enough for “Supported”. Where a feature depends on real hardware or real music and the only evidence is synthetic, it says **Unverified**."
      source={linkRepoFile(
        "This page is `docs/COMPATIBILITY.md`, rendered at build time. `docs/STATUS_AUDIT.md` is the evidence log, and it wins where the two disagree.",
        "docs/COMPATIBILITY.md",
      )}
      markdown={markdown}
      tocLabel="Areas"
    />
  );
}
