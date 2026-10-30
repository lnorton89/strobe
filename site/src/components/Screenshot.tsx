import Window from "@/components/Window";

/*
 * A real capture of the app, in the shared window chrome.
 *
 * The guide asks for "real output over mockups" and "authentic application
 * screenshots" (docs/branding.md sections 1 and 7), so every image this
 * shows comes from docs/images/, copied in by scripts/sync-assets.mjs. The
 * intrinsic size is passed through so the browser reserves the space before
 * the image arrives and nothing below it moves.
 */
export default function Screenshot({
  src,
  alt,
  width,
  height,
  path = "strobe.exe",
  title,
  priority,
  className,
}: {
  src: string;
  alt: string;
  width: number;
  height: number;
  /** Left of the bar, beside the mark. */
  path?: string;
  /** Centred: what the capture shows. */
  title: string;
  /** Loads eagerly, for the one image above the fold. */
  priority?: boolean;
  className?: string;
}) {
  return (
    <Window path={path} title={title} className={className}>
      {/* A static export has no image optimiser, so a plain <img> is the honest element. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src={src}
        alt={alt}
        width={width}
        height={height}
        loading={priority ? "eager" : "lazy"}
        fetchPriority={priority ? "high" : undefined}
        decoding="async"
        className="bg-code-ground block h-auto w-full"
      />
    </Window>
  );
}
