import type { Metadata } from "next";
import { Inter, JetBrains_Mono, Space_Grotesk } from "next/font/google";
import SiteHeader from "@/components/SiteHeader";
import SiteFooter from "@/components/SiteFooter";
import { og } from "./(home)/page";
import { ogImage } from "@/lib/og-image";
import { THEME_INIT_SCRIPT } from "@/lib/theme";
import { DESCRIPTION, SITE_URL, TAGLINE } from "@/lib/site";
import "./globals.css";

/*
 * The guide's pairing (docs/branding.md section 4): Space Grotesk for
 * headings and the wordmark, Inter for body copy and controls, JetBrains Mono
 * for shader code and technical values.
 */
const spaceGrotesk = Space_Grotesk({
  variable: "--font-space-grotesk",
  subsets: ["latin"],
  weight: ["500", "600", "700"],
  display: "swap",
});

const inter = Inter({
  variable: "--font-inter",
  subsets: ["latin"],
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  variable: "--font-jetbrains-mono",
  subsets: ["latin"],
  weight: ["400", "500"],
  display: "swap",
});

const TITLE = `Strobe — ${TAGLINE}`;

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: {
    default: TITLE,
    template: "%s · Strobe",
  },
  description: DESCRIPTION,
  applicationName: "Strobe",
  alternates: { canonical: "/" },
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
  openGraph: {
    siteName: "Strobe",
    title: TITLE,
    description: DESCRIPTION,
    type: "website",
    locale: "en_US",
    url: "/",
    images: [ogImage("/", og)],
  },
  twitter: {
    card: "summary_large_image",
    title: TITLE,
    description: DESCRIPTION,
    images: [ogImage("/", og)],
  },
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
        <SiteHeader />
        <main id="main-content" tabIndex={-1} className="flex-1 outline-none">
          {children}
        </main>
        <SiteFooter />
      </body>
    </html>
  );
}
