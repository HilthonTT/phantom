import type { Metadata } from "next";
import { Inter, JetBrains_Mono } from "next/font/google";

import { asset, site } from "@/lib/site";
import "./globals.css";

const inter = Inter({ subsets: ["latin"], variable: "--font-inter" });
const jetbrains = JetBrains_Mono({ subsets: ["latin"], variable: "--font-jetbrains" });

export const metadata: Metadata = {
  title: { default: `${site.name} — ${site.tagline}`, template: `%s · ${site.name}` },
  description:
    "phantom is a Matrix homeserver written in Rust, with a terminal admin console in Go: chat, users, rooms, federation, media, logs and abuse reports, all from the keyboard.",
  icons: { icon: asset("/favicon.svg") },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${inter.variable} ${jetbrains.variable}`}>
      <body className="min-h-screen font-sans antialiased">{children}</body>
    </html>
  );
}
