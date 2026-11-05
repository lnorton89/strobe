"use client";

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { Action } from "@/components/Action";
import { DownloadIcon } from "@/components/brand/icons";
import { sessionCache } from "@/lib/github";
import {
  fetchLatestRelease,
  formatBytes,
  parseRelease,
  readCachedRelease,
  releaseLabel,
  UNKNOWN_RELEASE,
  writeCachedRelease,
  type Release,
} from "@/lib/release";

/*
 * Everything on the site that names the release (lib/release.ts explains why it
 * is looked up twice).
 *
 * The provider starts from what the build wrote, so the server's HTML and the
 * first client render match and a reader with no JavaScript still gets a real
 * version and a working link. It then asks GitHub, at most once per session per
 * ten minutes, which is the same budget the roadmap's activity feed spends: one
 * of the reader's sixty unauthenticated requests an hour. Any failure leaves
 * the build's answer alone.
 */

const ReleaseContext = createContext<Release>(UNKNOWN_RELEASE);

export function ReleaseProvider({
  initial,
  children,
}: {
  initial: Release;
  children: ReactNode;
}) {
  const [release, setRelease] = useState(initial);

  useEffect(() => {
    const controller = new AbortController();
    const storage = sessionCache();
    const cached = readCachedRelease(storage, Date.now());
    // Through a promise either way, so the update is always in a callback: a
    // synchronous one in an effect body is a cascading render.
    const pending = cached
      ? Promise.resolve(cached)
      : fetchLatestRelease({ signal: controller.signal });
    void pending.then((payload) => {
      if (!payload || controller.signal.aborted) return;
      const fresh = parseRelease(payload);
      if (!fresh) return;
      setRelease(fresh);
      if (!cached) writeCachedRelease(storage, payload, Date.now());
    });
    return () => controller.abort();
  }, []);

  return <ReleaseContext.Provider value={release}>{children}</ReleaseContext.Provider>;
}

/** A link to the current release's page, reading its tag. */
export function ReleaseLink({
  suffix,
  className,
}: {
  /** Set after the tag, for example the release's stage. */
  suffix?: string;
  className?: string;
}) {
  const release = useContext(ReleaseContext);
  return (
    <a
      href={release.url}
      target="_blank"
      rel="noopener noreferrer"
      data-release
      className={className}
    >
      {releaseLabel(release)}
      {suffix ? ` · ${suffix}` : null}
    </a>
  );
}

/** The tag as plain text, for a label beside other words. */
export function ReleaseName() {
  return <span data-release>{releaseLabel(useContext(ReleaseContext))}</span>;
}

/** "<tag> is the current release." Nothing at all when no number is known. */
export function CurrentReleaseSentence() {
  const { tag } = useContext(ReleaseContext);
  return tag ? <span data-release>{tag} is the current release. </span> : null;
}

/**
 * The download page's primary action: the zip itself when GitHub listed it, with
 * what a reader wants to know before fetching it, otherwise the release's page.
 */
export function WindowsDownload() {
  const release = useContext(ReleaseContext);
  const { windows, checksum, published, tag, url } = release;
  const facts = [
    windows?.bytes ? formatBytes(windows.bytes) : undefined,
    published ? `published ${published}` : undefined,
  ].filter((fact) => fact !== undefined);

  return (
    <div data-release-download>
      <div className="flex flex-wrap items-center gap-4">
        {windows ? (
          <Action href={windows.url} variant="primary" icon={DownloadIcon} file>
            Download <span className="normal-case">{tag}</span> for Windows
          </Action>
        ) : (
          <Action href={url} variant="primary" icon={DownloadIcon} external>
            Download from GitHub
          </Action>
        )}
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-secondary hover:text-text panel-hover text-sm underline underline-offset-[3px]"
        >
          Release notes
        </a>
      </div>
      <p className="text-label mt-4 font-mono text-xs leading-relaxed wrap-anywhere">
        {windows?.name ?? "strobe-<version>-x86_64-pc-windows-msvc.zip"}
        {facts.map((fact) => ` · ${fact}`)}
        {checksum ? (
          <>
            {" · "}
            <a
              href={checksum.url}
              className="hover:text-text underline underline-offset-[3px]"
            >
              SHA-256
            </a>
          </>
        ) : null}
      </p>
    </div>
  );
}
