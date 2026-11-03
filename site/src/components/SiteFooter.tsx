import Link from "next/link";
import Wordmark from "@/components/brand/Wordmark";
import {
  DownloadIcon,
  GitHubIcon,
  GuideIcon,
  NAV_ICONS,
} from "@/components/brand/icons";
import { BADGES, DEFAULT_BRANCH, NAV, POSITIONING, RELEASES_URL, REPO_URL } from "@/lib/site";
import { CONTAINER, ICON } from "@/lib/constants";

/*
 * The documentation footer the guide asks for (section 2: "horizontal lockup
 * ... documentation footers"): the lockup and the positioning statement on
 * the left, a map of the site and the project's links on the right, and the
 * primary gradient once, as a hairline across the top.
 *
 * Every link is marked with the same icon the section uses elsewhere, so the
 * footer reads as a map of the site rather than as a list of words.
 */

const LINK =
  "text-secondary hover:text-text panel-hover flex items-center gap-2.5 text-sm";

const HEADING =
  "text-label mb-1 font-mono text-[10px] tracking-[0.16em] uppercase";

export default function SiteFooter() {
  return (
    <footer className="border-hairline border-t">
      <div aria-hidden="true" className="spectrum-rule h-px opacity-60" />
      <div className={`${CONTAINER} py-14`}>
        <div className="flex flex-col gap-12 lg:flex-row lg:justify-between">
          <div className="flex flex-col gap-5">
            <Wordmark size={30} withMark withDescriptor />
            <p className="text-secondary max-w-[46ch] text-sm leading-relaxed">
              {POSITIONING}
            </p>
            <p className="text-label font-mono text-[11px] tracking-[0.08em]">
              {BADGES.join(" · ")} · Windows first, Linux too
            </p>
          </div>

          <div className="flex flex-wrap gap-16">
            <nav aria-label="Footer" className="flex flex-col gap-3">
              <h2 className={HEADING}>Documentation</h2>
              {NAV.map((item) => {
                const Icon = NAV_ICONS[item.icon];
                return (
                  <Link key={item.href} href={item.href} className={LINK}>
                    <Icon size={ICON.inline} />
                    {item.label}
                  </Link>
                );
              })}
            </nav>

            <div className="flex flex-col gap-3">
              <h2 className={HEADING}>Project</h2>
              <a href={REPO_URL} target="_blank" rel="noopener noreferrer" className={LINK}>
                <GitHubIcon size={ICON.inline} />
                Repository
              </a>
              <a href={RELEASES_URL} target="_blank" rel="noopener noreferrer" className={LINK}>
                <DownloadIcon size={ICON.inline} />
                Releases
              </a>
              <a
                href={`${REPO_URL}/issues`}
                target="_blank"
                rel="noopener noreferrer"
                className={LINK}
              >
                <GitHubIcon size={ICON.inline} />
                Issues
              </a>
              <a
                href={`${REPO_URL}/blob/${DEFAULT_BRANCH}/CONTRIBUTING.md`}
                target="_blank"
                rel="noopener noreferrer"
                className={LINK}
              >
                <GuideIcon size={ICON.inline} />
                Contributing
              </a>
              <a
                href={`${REPO_URL}/blob/${DEFAULT_BRANCH}/LICENSE`}
                target="_blank"
                rel="noopener noreferrer"
                className={LINK}
              >
                <GuideIcon size={ICON.inline} />
                License
              </a>
            </div>
          </div>
        </div>

        <div className="border-hairline mt-14 flex flex-col gap-3 border-t pt-8 sm:flex-row sm:items-center sm:justify-between">
          <span className="text-label font-mono text-[11px] tracking-[0.16em] uppercase">
            Strobe · open source · MIT
          </span>
          <span className="text-label font-mono text-[11px]">
            {REPO_URL.replace("https://", "")}
          </span>
        </div>
      </div>
    </footer>
  );
}
