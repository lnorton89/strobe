import Markdown from "@/components/Markdown";
import TableOfContents from "@/components/TableOfContents";
import { extractHeadings } from "@/lib/markdown-toc";

/*
 * The layout for a long repository document.
 *
 * These files run to hundreds of lines — the user guide and the roadmap are
 * both book-length by web standards — and as a single column they read as a
 * wall. A contents rail beside the prose turns the same text into something
 * navigable, and gives the reader a sense of how much is left.
 *
 * The rail only appears where there is room for it. Below the large breakpoint
 * the same list sits above the document, closed, since a sticky sidebar on a
 * phone would cost more space than it earns and a forty-screen page with no
 * contents is a page nobody finds anything in.
 *
 * Paragraphs and list items keep to a readable measure; tables, figures and code
 * windows may use the whole column.
 */
export default function DocBody({
  markdown,
  tocLabel,
}: {
  markdown: string;
  tocLabel?: string;
}) {
  const headings = extractHeadings(markdown, { minDepth: 2, maxDepth: 2 });

  return (
    <div className="mx-auto grid max-w-[1400px] gap-x-16 gap-y-10 px-6 sm:px-10 lg:grid-cols-[minmax(0,1fr)_240px]">
      {/*
        The prose comes first in the source so it is what a screen reader and a
        keyboard reach first; the rail is placed to its right visually.
      */}
      {headings.length > 0 ? (
        <details className="border-hairline bg-panel border lg:hidden">
          <summary className="text-text cursor-pointer px-5 py-3.5 text-sm font-semibold">
            {tocLabel ?? "On this page"}
          </summary>
          <ul className="border-hairline flex list-none flex-col border-t px-5 py-3">
            {headings.map((entry) => (
              <li key={entry.slug}>
                <a
                  href={`#${entry.slug}`}
                  className="text-secondary hover:text-text block py-1.5 text-[14px] leading-snug"
                >
                  {entry.text}
                </a>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
      <div className="min-w-0 lg:order-1 [&_li]:max-w-[78ch] [&_p]:max-w-[78ch]">
        <Markdown>{markdown}</Markdown>
      </div>
      <aside className="hidden min-w-0 lg:order-2 lg:block">
        <TableOfContents entries={headings} label={tocLabel} />
      </aside>
    </div>
  );
}
