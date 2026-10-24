import type { ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeSlug from "rehype-slug";
import AsciiDiagram from "@/components/AsciiDiagram";
import { PlainWindow } from "@/components/ShellBlock";
import { isAsciiDiagram } from "@/lib/ascii-diagram";

/*
 * Renders markdown in the site's type scale (guide section 4).
 *
 * Nothing here sets a measure. Line length is a layout decision that belongs
 * to the page — a lede beside a heading, a full-width document body and a
 * caption in a narrow column all want different widths from the same
 * component, so each block element fills whatever container it is given.
 *
 * Fenced blocks are set in mono on the dark code ground, without syntax
 * colour: the guide reserves the saturated accents for icons, focus and
 * short labels (section 3), and a page of rainbow GLSL would spend them all.
 *
 * `rehype-slug` supplies the heading ids the documents' own cross-links
 * (`USER_GUIDE.md#linux`) depend on.
 */

/**
 * Titles a fenced block.
 *
 * The documents tag blocks as `bash`, `toml`, `glsl` and so on, and some not
 * at all. Whatever the fence says becomes the window title, so a reader can
 * tell a shell session from a playlist file without the site inventing a
 * colour theme for each language.
 *
 * Blocks that draw a figure never reach this: they are picked off above by
 * `isAsciiDiagram` and rendered as figures rather than as windows.
 */
function plainTitle(language: string | undefined): { path: string; title: string } {
  // An untagged block in these documents is as often pseudo-code or a file
  // layout as program output, so it is not called "output".
  if (!language || language === "text") {
    return { path: "text", title: "Plain text" };
  }
  const titles: Record<string, string> = {
    bash: "Shell",
    sh: "Shell",
    console: "Shell",
    powershell: "PowerShell",
    ps1: "PowerShell",
    glsl: "Shader",
    js: "Script",
    javascript: "Script",
    rust: "Rust",
    toml: "Configuration",
    json: "Configuration",
  };
  return { path: language, title: titles[language] ?? language.toUpperCase() };
}

const INLINE_CODE =
  "border-hairline bg-panel text-bright border px-[5px] py-[2px] font-mono text-[0.875em] break-words";

/** Shared renderers; the two entry points differ only in block spacing. */
function components(inline: boolean): Components {
  return {
    h2: ({ children, id }) => (
      <h2
        id={id}
        className="border-hairline text-text mt-20 scroll-mt-24 border-t pt-10 text-[28px] leading-tight font-semibold tracking-tight wrap-break-word first:mt-0 sm:text-[34px]"
      >
        {children}
      </h2>
    ),
    h3: ({ children, id }) => (
      <h3
        id={id}
        className="text-text mt-12 scroll-mt-24 text-[19px] leading-snug font-semibold tracking-[-0.015em] wrap-break-word sm:text-[22px]"
      >
        {children}
      </h3>
    ),
    h4: ({ children, id }) => (
      <h4
        id={id}
        className="text-bright mt-10 scroll-mt-24 text-[16px] font-semibold tracking-[-0.01em] wrap-break-word"
      >
        {children}
      </h4>
    ),
    p: ({ children }) =>
      inline ? (
        <p className="not-first:mt-4">{children}</p>
      ) : (
        <p className="text-secondary mt-5 text-[16.5px] leading-[1.75] wrap-break-word text-pretty">
          {children}
        </p>
      ),
    a: ({ children, href }) => {
      const external = href?.startsWith("http");
      return (
        <a
          href={href}
          target={external ? "_blank" : undefined}
          rel={external ? "noopener noreferrer" : undefined}
          className="text-accent-text decoration-accent-text/40 hover:decoration-accent-text underline underline-offset-[3px]"
        >
          {children}
        </a>
      );
    },
    ul: ({ children }) => (
      <ul className="mt-5 flex list-none flex-col gap-3 pl-0">{children}</ul>
    ),
    ol: ({ children }) => (
      <ol className="text-secondary marker:text-label mt-5 flex list-decimal flex-col gap-3 pl-6 marker:font-mono marker:text-[13px]">
        {children}
      </ol>
    ),
    li: ({ children, className }) =>
      // A task item carries its own box, so it does without the bullet.
      className?.includes("task-list-item") ? (
        <li className="text-secondary text-[16.5px] leading-[1.7] wrap-break-word">
          {children}
        </li>
      ) : (
        <li className="text-secondary relative pl-6 text-[16.5px] leading-[1.7] wrap-break-word before:absolute before:top-[0.62em] before:left-0 before:h-[6px] before:w-[6px] before:bg-hairline-strong before:content-[''] in-[ol]:pl-0 in-[ol]:before:hidden">
          {children}
        </li>
      ),
    strong: ({ children }) => (
      <strong className="text-text font-semibold">{children}</strong>
    ),
    em: ({ children }) => <em className="text-bright not-italic">{children}</em>,
    blockquote: ({ children }) => (
      <blockquote className="border-accent text-bright my-8 border-l-2 pl-6 [&>p]:text-[17px]">
        {children}
      </blockquote>
    ),
    /*
     * A task-list checkbox (`- [x]` in the roadmap). remark-gfm renders it as
     * a disabled <input> with no label, which a screen reader announces as an
     * unnamed control. It is a status, not a control, so it is drawn as one:
     * a small box, filled in the accent when done, with its state as text.
     */
    input: ({ type, checked }) =>
      type === "checkbox" ? (
        <span
          role="img"
          aria-label={checked ? "Done" : "Not done"}
          className={`mr-2 inline-block h-[11px] w-[11px] translate-y-px border ${
            checked ? "border-accent bg-accent" : "border-hairline-strong"
          }`}
        />
      ) : null,
    hr: () => <hr className="border-hairline my-16 border-t" />,
    img: ({ src, alt }) => (
      // A figure from the repository, served as a plain file: a static export
      // has no image optimiser for next/image to call.
      // eslint-disable-next-line @next/next/no-img-element
      <img
        src={typeof src === "string" ? src : undefined}
        alt={alt ?? ""}
        loading="lazy"
        decoding="async"
        className="border-hairline bg-code-ground my-8 block h-auto max-w-full border"
      />
    ),
    table: ({ children }) => (
      // Focusable, so a table that scrolls sideways on a phone can be
      // scrolled from the keyboard.
      <div tabIndex={0} className="rule-grid my-8 block overflow-x-auto">
        <table className="bg-ground w-full border-collapse text-left">{children}</table>
      </div>
    ),
    thead: ({ children }) => <thead className="bg-panel">{children}</thead>,
    th: ({ children }) => (
      <th className="border-hairline text-label border-b px-5 py-3 font-mono text-[10px] font-medium tracking-[0.16em] whitespace-nowrap uppercase">
        {children}
      </th>
    ),
    td: ({ children }) => (
      <td className="border-hairline text-secondary border-b px-5 py-3 align-top text-[14px] leading-relaxed wrap-break-word">
        {children}
      </td>
    ),
    code: ({ className, children }) => {
      const text = String(children ?? "").replace(/\n$/, "");
      const language = /language-([\w-]+)/.exec(className ?? "")?.[1];
      // Inline code never spans lines; a fenced block either declares a
      // language or contains a newline. That distinction is enough, and avoids
      // depending on the `inline` prop react-markdown dropped.
      const isBlock = language !== undefined || text.includes("\n");

      if (!isBlock) return <code className={INLINE_CODE}>{children}</code>;
      /*
       * A block drawn from box characters is a figure, not output. It is
       * recognised by its characters rather than by its text, so an edit
       * upstream cannot quietly turn it back into a terminal window — and if
       * it stops being a drawing, it falls through to the window below.
       */
      if (isAsciiDiagram(text)) {
        return <AsciiDiagram text={text} className="my-8" />;
      }
      const plain = plainTitle(language);
      return (
        <PlainWindow
          text={text}
          path={plain.path}
          title={plain.title}
          className="my-8"
        />
      );
    },
    // `code` above renders the whole block, so the wrapper is redundant.
    pre: ({ children }) => <>{children}</>,
  };
}

const BLOCK_COMPONENTS = components(false);
const INLINE_COMPONENTS = components(true);

/** Document-style markdown, with its own type scale and vertical rhythm. */
export default function Markdown({ children }: { children: string }): ReactNode {
  return (
    <div className="min-w-0">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSlug]}
        components={BLOCK_COMPONENTS}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}

/**
 * Markdown that inherits the surrounding typography.
 *
 * For copy that is styled by its container — a hero lede at 27px, a caption at
 * 13px — where the point of using markdown is emphasis and inline code, not a
 * type scale of its own.
 */
export function InlineMarkdown({ children }: { children: string }): ReactNode {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeSlug]}
      components={INLINE_COMPONENTS}
    >
      {children}
    </ReactMarkdown>
  );
}
