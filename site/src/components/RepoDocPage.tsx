import type { ComponentType, ReactNode } from "react";
import DocBody from "@/components/DocBody";
import PageHeader from "@/components/PageHeader";
import { SourceNote } from "@/components/ui";
import { CONTAINER } from "@/lib/constants";

/*
 * A page that is one of the repository's own documents.
 *
 * The header and the lede are the site's; the body is the document, read at
 * build time by lib/repo-docs.ts, with a contents rail beside it. The source
 * note comes after the document, for a reader who wants to propose a change:
 * above it, a build detail was the first thing every reader had to read past.
 */
export default function RepoDocPage({
  section,
  note,
  icon,
  title,
  lede,
  source,
  markdown,
  tocLabel,
  children,
}: {
  section: string;
  note: string;
  icon: ComponentType<{ size?: number; className?: string }>;
  title: string;
  lede: string;
  /** Markdown naming the source file, already linked. */
  source: string;
  markdown: string;
  tocLabel?: string;
  /** Anything the page shows between the header and the document. */
  children?: ReactNode;
}) {
  return (
    <article className="pb-28">
      <PageHeader section={section} note={note} icon={icon} title={title} lede={lede} />
      {children}
      <div className="pt-14">
        <DocBody markdown={markdown} tocLabel={tocLabel} />
      </div>
      <div className={`${CONTAINER} pt-16`}>
        <SourceNote>{source}</SourceNote>
      </div>
    </article>
  );
}
