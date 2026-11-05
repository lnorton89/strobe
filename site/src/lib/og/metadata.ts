/*
 * A page's metadata, with its social card.
 *
 * Next shallow-merges metadata, so a page that sets `openGraph` replaces the
 * layout's whole object rather than adding to it. Every page therefore goes
 * through this one function, which states the fields a preview reads: pages
 * that spread only the images used to lose `og:site_name`, `og:type` and
 * `og:url` without anything saying so.
 */

import type { Metadata } from "next";
import type { ExportedRoute } from "../site";
import { CARDS } from "./cards";
import { og } from "./plate";

export function pageMetadata(
  route: ExportedRoute,
  page: {
    /** The page's `<title>`, before the layout's " · Strobe". */
    title: NonNullable<Metadata["title"]>;
    description: string;
  },
): Metadata {
  const card = CARDS[route];
  const social = og.metadata(route, card);
  return {
    title: page.title,
    description: page.description,
    alternates: { canonical: route },
    openGraph: {
      siteName: "Strobe",
      type: "website",
      locale: "en_US",
      url: route,
      title: card.title,
      description: page.description,
      images: social.openGraph.images,
    },
    twitter: {
      ...social.twitter,
      title: card.title,
      description: page.description,
    },
  };
}
