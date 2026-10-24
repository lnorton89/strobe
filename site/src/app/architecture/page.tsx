import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { ArchitectureIcon } from "@/components/brand/icons";
import { ogImageMetadata } from "@/lib/og-image";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Architecture",
  title: "Fifteen crates, one GPU boundary.",
  description:
    "Only the app knows about everything, only strobe-gfx touches the GPU, and everything that can be tested without a GPU is.",
  alt: "Strobe social card — the architecture.",
};

export const metadata: Metadata = {
  title: "Architecture",
  description:
    "How Strobe is built: the crate graph, the GL boundary and its context providers, a frame end to end, context loss, the scene player, shader assembly, playlists and Shadertoy import.",
  alternates: { canonical: "/architecture/" },
  ...ogImageMetadata("/architecture/", og),
};

export default async function ArchitecturePage() {
  const markdown = await readRepoDoc("docs/ARCHITECTURE.md");
  return (
    <RepoDocPage
      section="Architecture"
      note="Native OpenGL via glow · Rust"
      icon={ArchitectureIcon}
      title="Fifteen crates, one GPU boundary."
      lede="Only the app knows about everything. Only `strobe-gfx` touches the GPU, and its `unsafe` lives in one directory. Everything that can be tested without a GPU is."
      source={linkRepoFile(
        "This page is `docs/ARCHITECTURE.md`, rendered at build time. The reasons behind each choice are in `docs/DECISIONS.md`.",
        "docs/ARCHITECTURE.md",
      )}
      markdown={markdown}
      tocLabel="Contents"
    />
  );
}
