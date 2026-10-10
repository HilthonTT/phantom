export const site = {
  name: "phantom",
  tagline: "A Matrix homeserver with a terminal admin console",
  repo: "https://github.com/HilthonTT/phantom",
  install: "git clone https://github.com/HilthonTT/phantom && cd phantom && make build",
};

/** Prefixes public assets with the deploy base path (GitHub Pages). */
export function asset(path: string): string {
  const base = process.env.NEXT_PUBLIC_BASE_PATH ?? "";
  return `${base}${path.startsWith("/") ? path : `/${path}`}`;
}
