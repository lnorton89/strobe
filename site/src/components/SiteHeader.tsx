import { GitHubIcon } from "@/components/brand/icons";
import NavLinks from "@/components/NavLinks";
import MobileMenu from "@/components/MobileMenu";
import SiteHeaderLogo from "@/components/SiteHeaderLogo";
import ThemeToggle from "@/components/ThemeToggle";
import { latestRelease, releaseLabel } from "@/lib/release";
import { REPO_URL } from "@/lib/site";

/*
 * Mark and wordmark, uppercase nav, and the version chip pinned to the right.
 * A hairline underneath, nothing else: the header carries no accent except the
 * mark's own gradient and the cyan underline on the current section (guide
 * section 6: cyan for active selection, not for every clickable element).
 */
export default async function SiteHeader() {
  const release = await latestRelease();
  return (
    <header className="border-hairline bg-ground/95 sticky top-0 z-50 border-b backdrop-blur-sm">
      <a
        href="#main-content"
        className="bg-accent text-on-accent sr-only px-4 py-2 text-sm font-semibold tracking-widest uppercase focus:not-sr-only focus:fixed focus:top-4 focus:left-6 focus:z-100"
      >
        Skip to content
      </a>

      <div className="mx-auto flex h-16 max-w-[1400px] items-center gap-8 px-6 sm:px-10">
        <SiteHeaderLogo />

        <nav aria-label="Primary" className="hidden items-center gap-8 lg:flex">
          <NavLinks />
        </nav>

        <div className="ml-auto flex items-center gap-3">
          <a
            href={release.url}
            target="_blank"
            rel="noopener noreferrer"
            className="border-hairline text-label hover:text-text hover:border-hairline-strong hidden border px-3 py-1.5 font-mono text-xs md:inline-block"
          >
            {releaseLabel(release)}
          </a>
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="Strobe on GitHub"
            className="border-hairline-strong text-text hover:border-text hover:bg-panel panel-hover pressable hidden h-9 items-center gap-2 border px-3 text-xs font-semibold tracking-widest uppercase sm:inline-flex"
          >
            <GitHubIcon size={14} />
            GitHub
          </a>
          <ThemeToggle />
          <MobileMenu />
        </div>
      </div>
    </header>
  );
}
