/** Every route in the static export, shared by the e2e specs. */
export const ROUTES = [
  { name: "home", path: "/", heading: /Strobe/i, og: "/og-image", nav: false },
  { name: "download", path: "/download/", heading: /Unzip it and run it/, og: "/download/og-image", nav: true },
  { name: "guide", path: "/guide/", heading: /Running a show with Strobe/, og: "/guide/og-image", nav: true },
  { name: "scenes", path: "/scenes/", heading: /Original scenes/, og: "/scenes/og-image", nav: true },
  { name: "compatibility", path: "/compatibility/", heading: /What works, where/, og: "/compatibility/og-image", nav: true },
  { name: "architecture", path: "/architecture/", heading: /one GPU boundary/, og: "/architecture/og-image", nav: true },
  { name: "roadmap", path: "/roadmap/", heading: /What is built/, og: "/roadmap/og-image", nav: true },
] as const;

export const NAV_ROUTES = ROUTES.filter((route) => route.nav);
