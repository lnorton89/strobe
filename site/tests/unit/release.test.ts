import { describe, expect, it } from "vitest";
import {
  compactRelease,
  fetchLatestRelease,
  formatBytes,
  parseRelease,
  readCachedRelease,
  RELEASE_CACHE_KEY,
  releaseEndpoint,
  releaseLabel,
  resolveRelease,
  UNKNOWN_RELEASE,
  writeCachedRelease,
} from "../../src/lib/release";
import { LATEST_RELEASE_URL, RELEASES_URL } from "../../src/lib/site";

/*
 * The release the site names comes from GitHub (lib/release.ts), never a
 * constant. These pin how the payload is read, the order of the build's lookups,
 * and what the site says when nothing answers. The tag is a made-up one on
 * purpose: nothing here should need editing when a release is cut.
 */

const PAYLOAD = {
  tag_name: "v9.8.7",
  html_url: "https://example.com/ignored",
  published_at: "2099-03-04T22:57:44Z",
  assets: [
    { name: "strobe-v9.8.7-sbom.cdx.json", size: 10 },
    { name: "strobe-v9.8.7-x86_64-pc-windows-msvc-symbols.zip", size: 20 },
    { name: "strobe-v9.8.7-x86_64-pc-windows-msvc.zip", size: 18_587_204 },
    { name: "strobe-v9.8.7-x86_64-pc-windows-msvc.zip.sha256", size: 107 },
  ],
};

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
}

describe("the latest release", () => {
  it("reads the tag, the date and the Windows zip from GitHub's payload", () => {
    const release = parseRelease(PAYLOAD);
    expect(release).toEqual({
      tag: "v9.8.7",
      url: `${RELEASES_URL}/tag/v9.8.7`,
      published: "2099-03-04",
      windows: {
        name: "strobe-v9.8.7-x86_64-pc-windows-msvc.zip",
        url: `${RELEASES_URL}/download/v9.8.7/strobe-v9.8.7-x86_64-pc-windows-msvc.zip`,
        bytes: 18_587_204,
      },
      checksum: {
        name: "strobe-v9.8.7-x86_64-pc-windows-msvc.zip.sha256",
        url: `${RELEASES_URL}/download/v9.8.7/strobe-v9.8.7-x86_64-pc-windows-msvc.zip.sha256`,
        bytes: 107,
      },
    });
    expect(release && releaseLabel(release)).toBe("v9.8.7");
  });

  it("adds the v a bare tag lacks, and keeps the real tag in its URLs", () => {
    const release = parseRelease({ tag_name: "9.8.7" });
    expect(release?.tag).toBe("v9.8.7");
    expect(release?.url).toBe(`${RELEASES_URL}/tag/9.8.7`);
  });

  it("refuses a payload with no version tag", () => {
    expect(parseRelease({ message: "Not Found" })).toBeUndefined();
    expect(parseRelease({ tag_name: "nightly" })).toBeUndefined();
    expect(parseRelease(null)).toBeUndefined();
  });

  it("does not take the symbols zip for the download", () => {
    const release = parseRelease({
      tag_name: "v9.8.7",
      assets: [{ name: "strobe-v9.8.7-x86_64-pc-windows-msvc-symbols.zip", size: 20 }],
    });
    expect(release?.windows).toBeUndefined();
    expect(release?.checksum).toBeUndefined();
  });

  it("builds every link itself, so a stored payload cannot choose an href", () => {
    const release = parseRelease({
      tag_name: "v9.8.7",
      html_url: "https://example.com/page",
      assets: [
        {
          name: "strobe-v9.8.7-x86_64-pc-windows-msvc.zip",
          browser_download_url: "https://example.com/zip",
        },
        { name: "../../evil", size: 1 },
      ],
    });
    expect(release?.url.startsWith(RELEASES_URL)).toBe(true);
    expect(release?.windows?.url.startsWith(RELEASES_URL)).toBe(true);
    expect(compactRelease({ tag_name: "v9.8.7", assets: [{ name: "../../evil" }] })?.assets).toEqual(
      [],
    );
  });

  it("names no number when nothing could say, and links to the latest release", () => {
    expect(releaseLabel(UNKNOWN_RELEASE)).toBe("Latest release");
    expect(UNKNOWN_RELEASE.url).toBe(LATEST_RELEASE_URL);
  });

  it("reads sizes as a browser's download list does", () => {
    expect(formatBytes(18_587_204)).toBe("18.6 MB");
  });
});

describe("the build's lookup", () => {
  const github = (read: () => unknown) => ({ name: "GitHub", read });
  const git = { name: "git describe", read: () => ({ tag_name: "v1.2.3" }) };

  it("prefers GitHub", async () => {
    const found = await resolveRelease([github(() => PAYLOAD), git]);
    expect(found?.source).toBe("GitHub");
    expect(found?.payload.tag_name).toBe("v9.8.7");
  });

  it("falls back to the checkout's tag when GitHub cannot be reached", async () => {
    const warnings: string[] = [];
    const found = await resolveRelease(
      [
        github(() => {
          throw new Error("offline");
        }),
        git,
      ],
      (message) => warnings.push(message),
    );
    expect(found?.source).toBe("git describe");
    expect(parseRelease(found?.payload)?.url).toBe(`${RELEASES_URL}/tag/v1.2.3`);
    expect(warnings.join()).toMatch(/offline/);
  });

  it("falls back when GitHub answers without a version", async () => {
    const found = await resolveRelease([github(() => ({ message: "rate limited" })), git]);
    expect(found?.source).toBe("git describe");
  });

  it("answers nothing from a shallow clone with GitHub down, without failing", async () => {
    const found = await resolveRelease([
      github(() => {
        throw new Error("offline");
      }),
      {
        name: "git describe",
        read: () => {
          throw new Error("fatal: No names found");
        },
      },
    ]);
    expect(found).toBeUndefined();
  });

  it("can be pointed at another API, and names the repository once", () => {
    expect(releaseEndpoint("http://127.0.0.1:9")).toMatch(
      /^http:\/\/127\.0\.0\.1:9\/repos\/[^/]+\/[^/]+\/releases\/latest$/,
    );
  });
});

describe("the browser's lookup", () => {
  it("answers nothing, rather than throwing, for each way GitHub can fail", async () => {
    const answers = [
      async () => new Response("{}", { status: 403 }),
      async () => new Response("not json"),
      async () => new Response(JSON.stringify({ message: "Not Found" })),
      async () => {
        throw new TypeError("network");
      },
    ];
    for (const fetchImpl of answers) {
      expect(await fetchLatestRelease({ fetchImpl })).toBeUndefined();
    }
  });

  it("reads a good answer", async () => {
    const payload = await fetchLatestRelease({
      fetchImpl: async () => new Response(JSON.stringify(PAYLOAD)),
    });
    expect(payload?.tag_name).toBe("v9.8.7");
  });

  it("caches an answer for ten minutes", () => {
    const storage = memoryStorage();
    const payload = compactRelease(PAYLOAD);
    if (!payload) throw new Error("the fixture should parse");
    writeCachedRelease(storage, payload, 1000);
    expect(readCachedRelease(storage, 1000 + 60_000)?.tag_name).toBe("v9.8.7");
    expect(readCachedRelease(storage, 1000 + 11 * 60_000)).toBeUndefined();
  });

  it("treats a stored entry as untrusted", () => {
    const storage = memoryStorage();
    storage.setItem(RELEASE_CACHE_KEY, "{ not json");
    expect(readCachedRelease(storage, 0)).toBeUndefined();
    storage.setItem(RELEASE_CACHE_KEY, JSON.stringify({ at: 0, payload: { tag_name: "<script>" } }));
    expect(readCachedRelease(storage, 0)).toBeUndefined();
  });
});
