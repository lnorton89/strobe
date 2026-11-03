import type { Metadata } from "next";
import type { ComponentType, ReactNode } from "react";
import DataTable from "@/components/DataTable";
import { InlineMarkdown } from "@/components/Markdown";
import PageHeader from "@/components/PageHeader";
import Reveal from "@/components/Reveal";
import SectionHeading from "@/components/SectionHeading";
import ShellBlock from "@/components/ShellBlock";
import { Action, ArrowLink, Callout, MarkedList, Prose } from "@/components/ui";
import {
  CompatibilityIcon,
  DownloadIcon,
  GuideIcon,
  ShellIcon,
} from "@/components/brand/icons";
import { ogImageMetadata } from "@/lib/og-image";
import { readPageContent } from "@/lib/page-content";
import { latestRelease, releaseLabel } from "@/lib/release";
import { DEFAULT_BRANCH, REPO_URL } from "@/lib/site";

/** Social image copy, rendered by ./og-image/route.tsx. */
export const og = {
  eyebrow: "Download",
  title: "Unzip it and run it.",
  description:
    "A Windows release with no installer and an ANGLE fallback beside it; Linux from source with a package list and a Nix flake.",
  alt: "Strobe social card — download and build.",
};

export const metadata: Metadata = {
  title: "Download",
  description:
    "Download Strobe for Windows, or build it on Linux from source with apt, dnf, pacman or the Nix flake. What the machine needs, and where Strobe keeps its settings.",
  alternates: { canonical: "/download/" },
  ...ogImageMetadata("/download/", og),
};

/** One part of the page, with its own section rule. */
function Step({
  title,
  note,
  icon,
  children,
}: {
  title: string;
  note?: string;
  icon?: ComponentType<{ size?: number; className?: string }>;
  children: ReactNode;
}) {
  return (
    <section className="min-w-0">
      <SectionHeading title={title} note={note} icon={icon} />
      <Reveal className="mt-9 min-w-0">{children}</Reveal>
    </section>
  );
}

/** Two columns that both clamp, so a wide transcript scrolls instead of pushing. */
function SplitStep({ children }: { children: ReactNode }) {
  return (
    <div className="grid gap-10 *:min-w-0 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] lg:items-start">
      {children}
    </div>
  );
}

const README_LINUX = `${REPO_URL}/blob/${DEFAULT_BRANCH}/README.md#linux`;

export default async function DownloadPage() {
  const [content, release] = await Promise.all([readPageContent("download"), latestRelease()]);

  return (
    <article className="pb-28">
      <PageHeader
        section="Download"
        note={`${releaseLabel(release)} · for testing, not production`}
        icon={DownloadIcon}
        title="Unzip it and run it."
        lede={content.block("lede")}
      />

      <div className="mx-auto flex max-w-350 flex-col gap-20 px-6 pt-16 *:min-w-0 sm:px-10">
        <Step title="Windows" note="x86_64 · no installer" icon={DownloadIcon}>
          <SplitStep>
            <div>
              <Prose>{content.block("windows")}</Prose>
              <div className="mt-8 flex flex-wrap gap-4">
                <Action href={release.url} variant="primary" icon={DownloadIcon} external>
                  GitHub Releases
                </Action>
                <Action href="/guide/" icon={GuideIcon}>
                  First run
                </Action>
              </div>
            </div>
            <Callout className="sm:p-9">
              <h3 className="text-text text-[18px] font-semibold tracking-[-0.01em]">
                What the machine needs
              </h3>
              <MarkedList items={content.list("requirements")} accent />
            </Callout>
          </SplitStep>
          <DataTable
            headings={["In the zip", "What it is"]}
            data={content.items("windows-files").map((item) => [
              item.title,
              <InlineMarkdown key={item.slug}>{item.body}</InlineMarkdown>,
            ])}
            className="mt-12"
          />
        </Step>

        <Step title="Linux" note="Build from source · X11 and Wayland" icon={ShellIcon}>
          <SplitStep>
            <div>
              <Prose>{content.block("linux")}</Prose>
              <Prose className="mt-6">{content.block("linux-runtime")}</Prose>
              <ArrowLink href={README_LINUX} external className="mt-6">
                Linux packages in the README
              </ArrowLink>
            </div>
            <ShellBlock
              title="Debian and Ubuntu"
              lines={[
                "sudo apt-get install -y build-essential pkg-config libasound2-dev",
                "cargo build --release -p strobe-app     # about 10 minutes on four cores",
                "target/release/strobe --diagnose        # GPU, driver and audio report",
              ]}
            />
          </SplitStep>
          <div className="mt-14">
            <SplitStep>
              <Prose>{content.block("nix")}</Prose>
              <ShellBlock
                title="Nix"
                lines={[
                  "nix develop                  # toolchain, ALSA, and the run-time libraries",
                  "cargo run --release -p strobe-app",
                  "nix run                      # or build the package and start it",
                ]}
              />
            </SplitStep>
          </div>
        </Step>

        <Step title="From source" note="Any platform with Rust" icon={ShellIcon}>
          <SplitStep>
            <Prose>{content.block("source")}</Prose>
            <ShellBlock
              title="Build and run"
              lines={[
                "git clone https://github.com/lnorton89/strobe",
                "cd strobe",
                "cargo run --release -p strobe-app",
                "cargo run --release -p strobe-app -- --playlist playlists/example.toml",
                "cargo run --release -p strobe-app -- --gl angle     # force ANGLE",
              ]}
            />
          </SplitStep>
        </Step>

        <Reveal>
          <Callout>
            <h2 className="text-text text-[24px] font-semibold tracking-tight">
              After installing
            </h2>
            <div className="text-secondary max-w-[80ch] text-[16px] leading-[1.7] [&_code]:font-mono [&_code]:text-[0.9em]">
              <InlineMarkdown>{content.block("after")}</InlineMarkdown>
            </div>
            <div className="flex flex-wrap gap-4">
              <Action href="/guide/" variant="primary" icon={GuideIcon}>
                User guide
              </Action>
              <Action href="/compatibility/" icon={CompatibilityIcon}>
                Compatibility
              </Action>
            </div>
          </Callout>
        </Reveal>
      </div>
    </article>
  );
}
