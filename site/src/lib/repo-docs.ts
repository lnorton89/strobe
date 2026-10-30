import { readFile } from "node:fs/promises";
import path from "node:path";
import { DEFAULT_BRANCH, REPO_URL } from "@/lib/site";

/*
 * The guide, compatibility, architecture and roadmap pages are the
 * repository's own markdown, read at build time from the checkout one level
 * above this app.
 *
 * They are not copied into the site. A copy would drift, and docs/ROADMAP.md
 * and docs/STATUS_AUDIT.md are updated by every change that lands — a stale
 * copy of a status document is worse than no copy. The build fails loudly if
 * a document is missing rather than rendering an empty page.
 */

const REPO_ROOT = path.join(process.cwd(), "..");

const BLOB_BASE = `${REPO_URL}/blob/${DEFAULT_BRANCH}/`;
const TREE_BASE = `${REPO_URL}/tree/${DEFAULT_BRANCH}/`;

/** Repository documents that have a page of their own on this site. */
export const DOC_ROUTES: Record<string, string> = {
  "docs/USER_GUIDE.md": "/guide/",
  "docs/COMPATIBILITY.md": "/compatibility/",
  "docs/ARCHITECTURE.md": "/architecture/",
  "docs/ROADMAP.md": "/roadmap/",
};

export type RepoDocName =
  | "docs/USER_GUIDE.md"
  | "docs/COMPATIBILITY.md"
  | "docs/ARCHITECTURE.md"
  | "docs/ROADMAP.md";

/**
 * Resolves `target` (as written inside `fromDoc`) to a path from the
 * repository root.
 *
 * The documents link relatively — docs/ROADMAP.md says `PLAN.md` and means
 * `docs/PLAN.md`; docs/USER_GUIDE.md says `../README.md` — so a link only
 * means something once it is resolved against the file it appears in.
 */
export function resolveRepoPath(fromDoc: string, target: string): string {
  const resolved = path.posix.normalize(
    path.posix.join(path.posix.dirname(fromDoc), target),
  );
  // A trailing slash says "directory", which normalize keeps but join may not.
  return target.endsWith("/") && !resolved.endsWith("/") ? `${resolved}/` : resolved;
}

/**
 * Rewrites the repo-relative links and images in a markdown document so they
 * resolve from the site.
 *
 * A document that has its own page here links to that page; everything else —
 * source files, other documents, directories — links into GitHub, because
 * there is nothing on this site to point at. An image in docs/images/ is
 * served from this site (scripts/sync-assets.mjs copies that folder), because
 * GitHub's raw host serves SVG as text/plain and an <img> will not draw it;
 * any other image links to GitHub's raw file. Absolute URLs and in-page
 * anchors are left alone.
 */
export function rewriteRepoLinks(markdown: string, fromDoc = "README.md"): string {
  return markdown.replace(
    /(!?)\[([^\]]*)\]\((?!https?:\/\/|#|mailto:)([^)\s]+)(\s+"[^"]*")?\)/g,
    (_match, bang: string, text: string, target: string, title: string | undefined) => {
      const [pathPart, anchor = ""] = target.split("#") as [string, string?];
      const clean = resolveRepoPath(fromDoc, pathPart.replace(/^\.\//, ""));
      const suffix = anchor ? `#${anchor}` : "";
      const titlePart = title ?? "";

      if (bang) {
        const local = /^docs\/images\/([^/]+)$/.exec(clean);
        const src = local
          ? `/generated/docs-images/${local[1]}`
          : `${REPO_URL}/raw/${DEFAULT_BRANCH}/${clean}`;
        return `![${text}](${src}${titlePart})`;
      }

      const ownPage = DOC_ROUTES[clean];
      if (ownPage) return `[${text}](${ownPage}${suffix}${titlePart})`;

      // A trailing slash means a directory, which GitHub serves under /tree.
      const base = clean.endsWith("/") ? TREE_BASE : BLOB_BASE;
      return `[${text}](${base}${clean}${suffix}${titlePart})`;
    },
  );
}

/**
 * Drops the document's leading H1. Each page renders its own header from the
 * site's type scale, so keeping the source H1 would show the title twice and
 * put two `<h1>`s on the page.
 */
export function stripLeadingHeading(markdown: string): string {
  return markdown.replace(/^#\s+.*(\r?\n)+/, "");
}

/** Reads a repository document and prepares it for rendering on the site. */
export async function readRepoDoc(name: RepoDocName): Promise<string> {
  const raw = await readFile(path.join(REPO_ROOT, name), "utf8");
  return rewriteRepoLinks(stripLeadingHeading(raw), name);
}

/**
 * Turns a backticked filename in prose into a link to it on GitHub.
 *
 * The route content files name their source as `` `docs/ROADMAP.md` `` rather
 * than spelling out a URL, so the copy stays readable and the repository URL
 * is not repeated in five places. Only the named file is linked; any other
 * inline code in the sentence is left alone.
 */
export function linkRepoFile(text: string, file: string): string {
  return text.replace(`\`${file}\``, `[${file}](${BLOB_BASE}${file})`);
}

/**
 * docs/ROADMAP.md opens with a note to the next contributor and a running
 * "Last updated" paragraph, both written for people working in the
 * repository. Its page starts at the first `##` section instead, and keeps the
 * date for its header.
 */
export function splitRoadmap(markdown: string): { body: string; updated?: string } {
  const updated = /_Last updated: (\d{4}-\d{2}-\d{2})/.exec(markdown)?.[1];
  const firstSection = markdown.search(/^## /m);
  return { body: firstSection >= 0 ? markdown.slice(firstSection) : markdown, updated };
}
