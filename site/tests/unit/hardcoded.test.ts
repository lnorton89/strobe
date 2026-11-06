import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/*
 * What must not be written into the site's own source (D557).
 *
 * The release comes from GitHub through lib/release.ts, so a version number
 * typed into a page goes stale the day the next release ships, as the site's
 * first constant did (D526). The repository's slug is written once, for the move
 * to another organisation. And the copy keeps to what is true: the project is
 * source-available, and it is described by what it does.
 *
 * Tests are not scanned, so their fixtures may name versions. The documents in
 * docs/ are not either: they are dated records, and a past release is a fact there.
 */

const SITE = process.cwd();

/** The source the site is built from: code, its copy, and the deploy scripts. */
function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((name) => {
    const file = path.join(directory, name);
    if (statSync(file).isDirectory()) return sourceFiles(file);
    return /\.(ts|tsx|mts|mjs|md|css|json)$/.test(name) ? [file] : [];
  });
}

const FILES = ["src", "scripts", "worker"].flatMap((directory) =>
  sourceFiles(path.join(SITE, directory)),
);

/** Every line matching `pattern`, as `file:line: text`. */
function matches(pattern: RegExp, files: readonly string[] = FILES): string[] {
  return files.flatMap((file) =>
    readFileSync(file, "utf8")
      .split("\n")
      .flatMap((line, index) =>
        pattern.test(line) ? [`${path.relative(SITE, file)}:${index + 1}: ${line.trim()}`] : [],
      ),
  );
}

/** Copy a reader sees: the content files, and the strings on the pages. */
const COPY_FILES = FILES.filter(
  (file) =>
    /content\.md$/.test(file) ||
    /(page|not-found|layout)\.tsx$/.test(file) ||
    /(cards|site)\.ts$/.test(file) ||
    /(SiteFooter|Release|FrameTimes)\.tsx$/.test(file),
);

describe("the site's source", () => {
  it("has files to scan", () => {
    expect(FILES.length, "the scan found no source").toBeGreaterThan(50);
    expect(COPY_FILES.length, "the scan found no copy").toBeGreaterThan(10);
  });

  it("names no release version; lib/release.ts reads them from GitHub", () => {
    const tag = /\bv\d+\.\d+(\.\d+)?\b/;
    const bare = /(?<![\d.@/#-])\b0\.\d+\.\d+\b(?![\d.])/;
    const asset = /strobe-v\d/;
    expect(
      [...matches(tag), ...matches(bare), ...matches(asset)].filter(
        // Icon path data is full of number runs that look like versions.
        (line) => !line.startsWith(path.join("src", "components", "brand", "icons.tsx")),
      ),
    ).toEqual([]);
  });

  it("does not version the site as if it were the release", () => {
    const manifest = JSON.parse(readFileSync(path.join(SITE, "package.json"), "utf8")) as {
      version: string;
    };
    expect(manifest.version, "site/package.json's version would read as Strobe's").toBe("0.0.0");
  });

  it("writes the repository's slug in one place, plus package.json", () => {
    const slug = /lnorton89\/strobe/;
    expect(matches(slug)).toEqual([expect.stringMatching(/^src[\\/]lib[\\/]site\.ts:/)]);
  });

  it("keeps package.json's repository the one site.ts names", () => {
    const site = readFileSync(path.join(SITE, "src", "lib", "site.ts"), "utf8");
    const repo = /export const REPO = "([^"]+)"/.exec(site)?.[1];
    expect(repo, "site.ts names no REPO").toBeDefined();
    const manifest = readFileSync(path.join(SITE, "package.json"), "utf8");
    expect(manifest.match(/github\.com\/[\w.-]+\/[\w.-]+?(?=[/.]|")/g)?.every((url) => url === `github.com/${repo}`)).toBe(true);
  });
});

describe("the site's copy", () => {
  it("does not call the project open source; it is source-available", () => {
    expect(matches(/open[- ]source/i, COPY_FILES)).toEqual([]);
    expect(matches(/\bMIT\b(?! with the Commons Clause)/, COPY_FILES)).toEqual([]);
  });

  it("states the hardware floor as an engineering fact, not a selling line", () => {
    expect(matches(/old (laptop|hardware|machine|computer)|bad hardware/i, COPY_FILES)).toEqual([]);
  });

  it("uses none of the words that say nothing", () => {
    const empty =
      /\b(powerful|seamless(ly)?|blazing|stunning|robust|effortless(ly)?|cutting[- ]edge|next[- ]generation|revolutionary|unleash|supercharge|world[- ]class|game[- ]chang\w+)\b/i;
    expect(matches(empty, COPY_FILES)).toEqual([]);
  });

  it("uses no em dash in what a reader sees", () => {
    const code = FILES.filter((file) => COPY_FILES.includes(file) && file.endsWith(".md"));
    expect(matches(/—/, code)).toEqual([]);
    // In code, only string and JSX text: comments may keep theirs.
    const strings = /^(?!\s*(\*|\/\/|\/\*)).*—/;
    expect(matches(strings, COPY_FILES.filter((file) => !file.endsWith(".md")))).toEqual([]);
  });
});
