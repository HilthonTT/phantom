"use client";

import { useState } from "react";

import { asset } from "@/lib/site";

const screens = [
  { id: "overview", label: "Overview", file: "overview.png", note: "The server at a glance" },
  { id: "users", label: "Users", file: "users.png", note: "Accounts, admins, last seen" },
  { id: "rooms", label: "Rooms", file: "rooms.png", note: "Members, versions, directory" },
  { id: "tokens", label: "Tokens", file: "tokens.png", note: "Registration tokens" },
  { id: "services", label: "Services", file: "services.png", note: "Every service and its worker" },
  { id: "media", label: "Media", file: "media.png", note: "The media store" },
  { id: "reports", label: "Reports", file: "reports.png", note: "Abuse reports users file" },
  { id: "logs", label: "Logs", file: "logs.png", note: "The server's recent log" },
  { id: "settings", label: "Settings", file: "settings.png", note: "The running config, masked" },
  { id: "help", label: "Help", file: "help.png", note: "Every key, one ? away" },
];

/** A screenshot of each console section, picked from a list. */
export function ScreenGallery() {
  const [active, setActive] = useState(screens[0]);
  return (
    <div className="grid gap-8 lg:grid-cols-[17rem_1fr]">
      <div className="flex gap-2 overflow-x-auto pb-2 lg:flex-col lg:overflow-visible lg:pb-0">
        {screens.map((s) => (
          <button
            key={s.id}
            type="button"
            onClick={() => setActive(s)}
            className={`flex shrink-0 flex-col rounded-xl border px-4 py-2.5 text-left text-sm transition ${
              active.id === s.id
                ? "border-layer-pink/50 bg-white/[0.06] text-white"
                : "border-white/5 text-mist-300 hover:border-white/15 hover:text-white"
            }`}
          >
            <span className="font-medium">{s.label}</span>
            <span className="text-xs text-mist-400">{s.note}</span>
          </button>
        ))}
      </div>
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        key={active.id}
        src={asset(`/media/${active.file}`)}
        alt={`The phantom console's ${active.label} section`}
        className="w-full rounded-2xl shadow-2xl ring-1 shadow-black/50 ring-white/10"
      />
    </div>
  );
}
