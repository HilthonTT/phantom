export type DocLink = { title: string; href: string };
export type DocSection = { title: string; links: DocLink[] };

/** Order of the docs sidebar and the previous/next links. */
export const docs: DocSection[] = [
  {
    title: "Getting started",
    links: [
      { title: "Introduction", href: "/docs/" },
      { title: "Installation", href: "/docs/installation/" },
      { title: "Quick start", href: "/docs/quick-start/" },
    ],
  },
  {
    title: "The console",
    links: [
      { title: "Signing in", href: "/docs/signing-in/" },
      { title: "Finding your way", href: "/docs/navigation/" },
      { title: "Chat", href: "/docs/chat/" },
      { title: "Server sections", href: "/docs/sections/" },
      { title: "Actions & commands", href: "/docs/actions/" },
      { title: "Key bindings", href: "/docs/keybindings/" },
    ],
  },
  {
    title: "The server",
    links: [
      { title: "Configuration", href: "/docs/configuration/" },
      { title: "Admin API", href: "/docs/admin-api/" },
      { title: "Status & limits", href: "/docs/status/" },
    ],
  },
  {
    title: "Contributing",
    links: [
      { title: "Architecture", href: "/docs/architecture/" },
      { title: "Development", href: "/docs/development/" },
      { title: "Recording media", href: "/docs/media/" },
    ],
  },
];

export const allDocs: DocLink[] = docs.flatMap((s) => s.links);
