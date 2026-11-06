import type { ReactNode } from "react";

/*
 * A table in the site's hairline grid.
 *
 * Two shapes:
 *
 * - `rows`: the common name/description pair, where the name is a row header.
 * - `headings` + `data`: an arbitrary grid, where the first cell of each row
 *   is still the row header.
 *
 * Either way the first column is a `<th scope="row">`, not a `<td>`: these
 * tables are lookups, and the first cell names the row.
 *
 * On a phone each row becomes a stack with each cell under its column's name.
 * Scrolling sideways instead left the column that mattered, usually the last,
 * off the screen with nothing saying it was there.
 */

/** A name and what it does. */
export type Row = { name: string; description: ReactNode };

const HEAD_CELL =
  "border-hairline text-label border-b px-5 py-3 font-mono text-[10px] font-medium tracking-[0.16em] uppercase";
const ROW_HEADER =
  "border-hairline text-text px-5 pt-4 text-left align-top font-mono text-[13px] font-normal max-sm:block sm:border-b sm:py-3.5 sm:whitespace-nowrap";
const CELL =
  "border-hairline text-secondary px-5 pt-2 align-top text-[14.5px] leading-relaxed last:pb-4 max-sm:block sm:border-b sm:py-3.5 max-sm:before:text-label max-sm:before:mb-1 max-sm:before:block max-sm:before:font-mono max-sm:before:text-[10px] max-sm:before:tracking-[0.14em] max-sm:before:uppercase max-sm:before:content-[attr(data-label)]";

type DataTableProps =
  | {
      rows: readonly Row[];
      nameHeading: string;
      headings?: never;
      data?: never;
      className?: string;
    }
  | {
      headings: readonly string[];
      data: readonly (readonly ReactNode[])[];
      rows?: never;
      nameHeading?: never;
      className?: string;
    };

export default function DataTable(props: DataTableProps) {
  const headings = props.rows
    ? [props.nameHeading, "What it does"]
    : props.headings;
  const data = props.rows
    ? props.rows.map((row) => [row.name, row.description] as const)
    : props.data;

  return (
    // Focusable and named: on a phone the table scrolls sideways, and a
    // scrolling region the keyboard cannot reach is a region it cannot read.
    <div
      tabIndex={0}
      role="region"
      aria-label={`Table: ${headings.join(", ")}`}
      className={`rule-grid mt-8 block overflow-x-auto ${props.className ?? ""}`}
    >
      <table className="bg-ground w-full border-collapse text-left max-sm:block">
        <thead className="bg-panel max-sm:sr-only">
          <tr>
            {headings.map((heading, index) => (
              <th
                key={heading}
                scope="col"
                className={`${HEAD_CELL}${index === 0 ? " whitespace-nowrap" : ""}`}
              >
                {heading}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="max-sm:block">
          {data.map((cells, rowIndex) => (
            <tr key={rowIndex} className="border-hairline max-sm:block max-sm:not-first:border-t">
              <th scope="row" className={ROW_HEADER}>
                {cells[0]}
              </th>
              {cells.slice(1).map((cell, cellIndex) => (
                <td key={cellIndex} data-label={headings[cellIndex + 1]} className={CELL}>
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
