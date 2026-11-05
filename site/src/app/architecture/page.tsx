import type { Metadata } from "next";
import RepoDocPage from "@/components/RepoDocPage";
import { ArchitectureIcon } from "@/components/brand/icons";
import { pageMetadata } from "@/lib/og/metadata";
import { linkRepoFile, readRepoDoc } from "@/lib/repo-docs";
import { readdirSync } from "node:fs";
import path from "node:path";

export const metadata: Metadata = pageMetadata("/architecture/", {
  title: "Architecture",
  description:
    "How Strobe is built: the crate graph, the GL boundary and its context providers, a frame end to end, context loss, the scene player, shader assembly, playlists and Shadertoy import.",
});

export default async function ArchitecturePage() {
  const markdown = await readRepoDoc("docs/ARCHITECTURE.md");
  // Counted, because the page used to say fifteen long after there were more.
  const crates = readdirSync(path.join(process.cwd(), "..", "crates")).length;
  return (
    <RepoDocPage
      section="Architecture"
      note={`${crates} crates · native OpenGL via glow`}
      icon={ArchitectureIcon}
      title="How Strobe is put together."
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
