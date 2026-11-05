import type { Metadata } from "next";
import localFont from "next/font/local";
import SiteHeader from "@/components/SiteHeader";
import SiteFooter from "@/components/SiteFooter";
import { ReleaseProvider } from "@/components/Release";
import { pageMetadata } from "@/lib/og/metadata";
import { buildRelease } from "@/lib/release-build";
import { THEME_INIT_SCRIPT } from "@/lib/theme";
import { DESCRIPTION, SITE_URL, TAGLINE } from "@/lib/site";
import "./globals.css";

/*
 * The guide's pairing (docs/branding.md section 4): Space Grotesk for
 * headings and the wordmark, Inter for body copy and controls, JetBrains Mono
 * for shader code and technical values.
 *
 * The files are @fontsource's, already a dependency for the social images
 * (lib/og-fonts.ts), so the build asks no font service for anything and the
 * pages load no font from anyone else's origin. All three are SIL OFL 1.1;
 * scripts/sync-assets.mjs ships their licences with the export. Only the
 * weights the pages set are listed: next/font preloads what is here.
 */
const spaceGrotesk = localFont({
  variable: "--font-space-grotesk",
  display: "swap",
  src: [
    { path: "../../node_modules/@fontsource/space-grotesk/files/space-grotesk-latin-500-normal.woff2", weight: "500" },
    { path: "../../node_modules/@fontsource/space-grotesk/files/space-grotesk-latin-600-normal.woff2", weight: "600" },
    { path: "../../node_modules/@fontsource/space-grotesk/files/space-grotesk-latin-700-normal.woff2", weight: "700" },
  ],
});

const inter = localFont({
  variable: "--font-inter",
  display: "swap",
  src: [
    { path: "../../node_modules/@fontsource/inter/files/inter-latin-400-normal.woff2", weight: "400" },
    { path: "../../node_modules/@fontsource/inter/files/inter-latin-500-normal.woff2", weight: "500" },
    { path: "../../node_modules/@fontsource/inter/files/inter-latin-600-normal.woff2", weight: "600" },
  ],
});

const jetbrainsMono = localFont({
  variable: "--font-jetbrains-mono",
  display: "swap",
  src: [
    { path: "../../node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2", weight: "400" },
    { path: "../../node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-500-normal.woff2", weight: "500" },
  ],
});

const TITLE = `Strobe — ${TAGLINE}`;

export const metadata: Metadata = {
  ...pageMetadata("/", {
    title: {
      default: TITLE,
      template: "%s · Strobe",
    },
    description: DESCRIPTION,
  }),
  metadataBase: new URL(SITE_URL),
  applicationName: "Strobe",
  icons: {
    icon: [
      { url: "/generated/brand/icon-small.svg", type: "image/svg+xml" },
      { url: "/generated/brand/icon-32.png", sizes: "32x32", type: "image/png" },
    ],
    apple: { url: "/generated/brand/icon-256.png", sizes: "256x256" },
  },
  keywords: [
    "Strobe",
    "VJ software",
    "shader visualizer",
    "Synesthesia",
    "Shadertoy",
    "GLSL",
    "audio reactive",
    "Rust",
    "OpenGL",
  ],
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html
      lang="en"
      className={`${inter.variable} ${spaceGrotesk.variable} ${jetbrainsMono.variable} h-full`}
    >
      <body className="bg-ground text-text flex min-h-full flex-col">
        {/*
          Applies a stored theme before first paint, so a visitor who chose a
          theme never sees the other one first.

          A plain inline script, not next/script: `beforeInteractive` is
          dropped entirely from a static export, which put the whole thing back
          to a flash on every load. As the first node in <body> this runs
          before any of the page below it is painted.
        */}
        <script dangerouslySetInnerHTML={{ __html: THEME_INIT_SCRIPT }} />
        <ReleaseProvider initial={buildRelease()}>
          <SiteHeader />
          <main id="main-content" tabIndex={-1} className="flex-1 outline-none">
            {children}
          </main>
          <SiteFooter />
        </ReleaseProvider>
      </body>
    </html>
  );
}
