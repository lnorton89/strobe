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
 * note says which file the page is, so a reader who wants to propose a change
 * knows where to make it.
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
      <div className={`${CONTAINER} pt-10`}>
        <SourceNote>{source}</SourceNote>
      </div>
      {children}
      <div className="pt-16">
        <DocBody markdown={markdown} tocLabel={tocLabel} />
      </div>
    </article>
  );
}
