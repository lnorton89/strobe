import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // The site is wholly static — every page is prerendered at build time from
  // the repository's own markdown — so it exports rather than running a
  // server, and Netlify publishes `out` directly.
  output: "export",
  // No image optimizer runs behind a static export.
  images: { unoptimized: true },
  // Emits `about/index.html` rather than `about.html`, so a directory URL
  // resolves without depending on the host's extension-guessing.
  trailingSlash: true,
  // Metaplate's renderer: a native binary and a Wasm file located from its own
  // path, neither of which Turbopack can bundle. The pages import the plate
  // for its metadata and never render through it (lib/og/plate.tsx).
  serverExternalPackages: ["@resvg/resvg-js", "satori"],
};

export default nextConfig;
