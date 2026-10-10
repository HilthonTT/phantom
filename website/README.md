# phantom website

The documentation site for phantom, built with Next.js (App Router, static export), Tailwind CSS and MDX.

```sh
npm install
npm run dev        # http://localhost:3000
npm run build      # static site in out/
```

- Pages live in `app/`; documentation pages are MDX files in `app/docs/<page>/page.mdx`.
- The docs sidebar order is in `lib/docs.ts`.
- GIFs and screenshots in `public/media/` are recorded from the real console against a scratch server. Run `make media` from the repository root to regenerate them.
- Pushing to `main` deploys to GitHub Pages through `.github/workflows/website.yml`. Set `NEXT_PUBLIC_BASE_PATH` when serving from a sub-path.
