import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { CompatibilityIcon } from "@/components/brand/icons";
import { ogImageMetadata } from "@/lib/og-image";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Compatibility",
  title: "What works, where, and the evidence.",
  description:
    "Supported, degraded, unsupported and unverified: the scene format, scripts, real Synesthesia scenes, Shadertoy, graphics backends, audio, playlists and show control.",
  alt: "Strobe social card — the compatibility matrix.",
};

export const metadata: Metadata = {
  title: "Compatibility",
  description:
    "Strobe's compatibility matrix: which SSF features, scripts, Synesthesia scenes, GPUs, drivers and controllers are supported, degraded or unverified, each with its evidence.",
  alternates: { canonical: "/compatibility/" },
  ...ogImageMetadata("/compatibility/", og),
};

export default async function CompatibilityPage() {
  const markdown = await readRepoDoc("docs/COMPATIBILITY.md");
  return (
    <RepoDocPage
      section="Compatibility"
      note="Every status points at its evidence"
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
