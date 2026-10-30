import { describe, expect, it } from "vitest";
import {
  linkRepoFile,
  resolveRepoPath,
  rewriteRepoLinks,
  splitRoadmap,
  stripLeadingHeading,
} from "@/lib/repo-docs";

const BLOB = "https://github.com/lnorton89/strobe/blob/master/";
const TREE = "https://github.com/lnorton89/strobe/tree/master/";

describe("resolveRepoPath", () => {
  it("resolves a sibling document against the folder it is written in", () => {
    expect(resolveRepoPath("docs/ROADMAP.md", "PLAN.md")).toBe("docs/PLAN.md");
  });

  it("climbs out of docs/ for a parent link", () => {
    expect(resolveRepoPath("docs/USER_GUIDE.md", "../README.md")).toBe("README.md");
  });

  it("keeps a directory's trailing slash", () => {
    expect(resolveRepoPath("README.md", "tools/userscript/")).toBe("tools/userscript/");
  });
});

describe("rewriteRepoLinks", () => {
  it("points documents that have their own page at that page", () => {
    expect(rewriteRepoLinks("see [ROADMAP.md](ROADMAP.md)", "docs/ARCHITECTURE.md")).toBe(
      "see [ROADMAP.md](/roadmap/)",
    );
    expect(rewriteRepoLinks("[guide](docs/USER_GUIDE.md)")).toBe("[guide](/guide/)");
    expect(
      rewriteRepoLinks("[matrix](COMPATIBILITY.md)", "docs/USER_GUIDE.md"),
    ).toBe("[matrix](/compatibility/)");
  });

  it("sends other files to the GitHub blob view, resolved from their document", () => {
    expect(rewriteRepoLinks("[D9](DECISIONS.md)", "docs/ROADMAP.md")).toBe(
      `[D9](${BLOB}docs/DECISIONS.md)`,
    );
    expect(rewriteRepoLinks("[readme](../README.md)", "docs/USER_GUIDE.md")).toBe(
      `[readme](${BLOB}README.md)`,
    );
  });

  it("sends directories to the tree view instead", () => {
    expect(rewriteRepoLinks("[playlists](playlists/)")).toBe(
      `[playlists](${TREE}playlists/)`,
    );
  });

  it("carries an anchor across the rewrite", () => {
    expect(rewriteRepoLinks("[linux](README.md#linux)")).toBe(
      `[linux](${BLOB}README.md#linux)`,
    );
    expect(rewriteRepoLinks("[keys](USER_GUIDE.md#keys)", "docs/ROADMAP.md")).toBe(
      "[keys](/guide/#keys)",
    );
  });

  it("leaves absolute links, anchors and mailto alone", () => {
    const untouched = [
      "[shadertoy](https://www.shadertoy.com/)",
      "[http](http://example.com)",
      "[section](#first-run)",
      "[mail](mailto:someone@example.com)",
    ].join("\n");
    expect(rewriteRepoLinks(untouched)).toBe(untouched);
  });

  it("normalises a leading ./", () => {
    expect(rewriteRepoLinks("[flake](./flake.nix)")).toBe(`[flake](${BLOB}flake.nix)`);
  });

  it("preserves a link title", () => {
    expect(rewriteRepoLinks('[flake](flake.nix "the dev shell")')).toBe(
      `[flake](${BLOB}flake.nix "the dev shell")`,
    );
  });

  it("serves a figure from docs/images/ from the site", () => {
    expect(
      rewriteRepoLinks("![arch](images/strobe-engine-architecture.svg)", "docs/ARCHITECTURE.md"),
    ).toBe("![arch](/generated/docs-images/strobe-engine-architecture.svg)");
  });

  it("links any other image to its raw file on GitHub", () => {
    expect(rewriteRepoLinks("![icon](assets/brand/icon-64.png)")).toBe(
      "![icon](https://github.com/lnorton89/strobe/raw/master/assets/brand/icon-64.png)",
    );
  });
});

describe("stripLeadingHeading", () => {
  it("removes the document title so the page owns its own h1", () => {
    expect(stripLeadingHeading("# Strobe user guide\n\nThis is for someone...")).toBe(
      "This is for someone...",
    );
  });

  it("leaves a document that does not start with a heading untouched", () => {
    expect(stripLeadingHeading("Intro paragraph\n\n# Later heading\n")).toBe(
      "Intro paragraph\n\n# Later heading\n",
    );
  });

  it("keeps every later heading", () => {
    expect(stripLeadingHeading("# Title\n\n## Install\n\nBody\n")).toBe("## Install\n\nBody\n");
  });
});

describe("splitRoadmap", () => {
  const roadmap = [
    "**Update this file whenever you finish something.**",
    "",
    "_Last updated: 2026-10-17 (a long running list of changes)._",
    "",
    "## Now",
    "",
    "Body",
  ].join("\n");

  it("starts the page at the first section", () => {
    expect(splitRoadmap(roadmap).body).toBe("## Now\n\nBody");
  });

  it("keeps the last-updated date for the header", () => {
    expect(splitRoadmap(roadmap).updated).toBe("2026-10-17");
  });

  it("leaves a document with no sections whole", () => {
    expect(splitRoadmap("Just prose.")).toEqual({ body: "Just prose.", updated: undefined });
  });
});

describe("linkRepoFile", () => {
  it("links the named file where it appears as inline code", () => {
    expect(linkRepoFile("This page is `docs/USER_GUIDE.md`.", "docs/USER_GUIDE.md")).toBe(
      `This page is [docs/USER_GUIDE.md](${BLOB}docs/USER_GUIDE.md).`,
    );
  });

  it("leaves other inline code in the sentence alone", () => {
    expect(linkRepoFile("`docs/ROADMAP.md` is read by `readRepoDoc`.", "docs/ROADMAP.md")).toBe(
      `[docs/ROADMAP.md](${BLOB}docs/ROADMAP.md) is read by \`readRepoDoc\`.`,
    );
  });

  it("is a no-op when the file is not mentioned", () => {
    expect(linkRepoFile("No filename here.", "docs/ROADMAP.md")).toBe("No filename here.");
  });

  it("links only the first mention, which is where the reference belongs", () => {
    const linked = linkRepoFile("`A.md` and `A.md`", "A.md");
    expect(linked.match(/https:/g), "more than one link").toHaveLength(1);
  });
});
