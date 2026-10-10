import Link from "next/link";
import {
  Activity,
  ArrowDownUp,
  ArrowRight,
  Flag,
  KeyRound,
  MessagesSquare,
  Radio,
  ScrollText,
  ServerCog,
  ShieldCheck,
  Users,
  Wrench,
} from "lucide-react";

import { CopyCommand } from "@/components/copy-command";
import { GithubIcon } from "@/components/github-icon";
import { Media } from "@/components/media";
import { ScreenGallery } from "@/components/screen-gallery";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { site } from "@/lib/site";

const haze = ["#f5c2e7", "#cba6f7", "#b4befe", "#89b4fa", "#74c7ec"];

/** Soft drifting bands behind the hero, like a ghost's trail. */
function Haze() {
  return (
    <div
      aria-hidden
      className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-[56rem] overflow-hidden [mask-image:linear-gradient(to_bottom,black_45%,transparent)]"
    >
      <div className="bg-grid absolute inset-0" />
      {haze.map((c, i) => (
        <div
          key={c}
          className="absolute left-1/2 h-40 w-[140%] -translate-x-1/2 rounded-[100%] blur-3xl"
          style={{
            top: `${6 + i * 7}rem`,
            background: c,
            opacity: 0.07 + (4 - i) * 0.012,
            transform: `translateX(-50%) rotate(${-4 + i * 2}deg)`,
          }}
        />
      ))}
    </div>
  );
}

function Section({
  id,
  eyebrow,
  title,
  intro,
  children,
}: {
  id?: string;
  eyebrow: string;
  title: React.ReactNode;
  intro?: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="mx-auto max-w-7xl scroll-mt-24 px-5 py-24">
      <p className="font-mono text-xs uppercase tracking-[0.25em] text-layer-pink">{eyebrow}</p>
      <h2 className="mt-3 max-w-3xl text-3xl font-semibold tracking-tight text-white sm:text-4xl">{title}</h2>
      {intro && <p className="mt-4 max-w-2xl text-lg leading-relaxed text-mist-300">{intro}</p>}
      <div className="mt-12">{children}</div>
    </section>
  );
}

function Showcase({
  title,
  text,
  media,
  alt,
  icon: Icon,
}: {
  title: string;
  text: string;
  media: string;
  alt: string;
  icon: React.ComponentType<{ className?: string }>;
}) {
  return (
    <div className="group flex flex-col overflow-hidden rounded-3xl border border-white/[0.07] bg-ink-850">
      <div className="p-7 pb-0">
        <span className="inline-flex rounded-xl bg-white/5 p-2.5 text-layer-sky ring-1 ring-white/10">
          <Icon className="size-5" />
        </span>
        <h3 className="mt-4 text-lg font-semibold text-white">{title}</h3>
        <p className="mt-1.5 text-sm leading-relaxed text-mist-300">{text}</p>
      </div>
      <div className="mt-6 flex-1 px-3 pb-3">
        <Media src={media} alt={alt} className="transition duration-500 group-hover:scale-[1.01]" />
      </div>
    </div>
  );
}

const capabilities = [
  {
    icon: Radio,
    title: "Live, not sampled",
    text: "Every section reads the running server. What isn't live yet says so in its footer.",
  },
  {
    icon: Users,
    title: "Accounts",
    text: "Reset passwords, grant or revoke admin, deactivate, sign devices out.",
  },
  {
    icon: KeyRound,
    title: "Registration tokens",
    text: "Create them with use limits and expiry, from a menu or one command.",
  },
  {
    icon: ShieldCheck,
    title: "Hard to lock yourself out",
    text: "No demoting yourself, no touching the server user, no deleting the admin room.",
  },
  {
    icon: Flag,
    title: "Abuse reports",
    text: "Reports users file are stored, posted to the admin room, and dismissed here.",
  },
  {
    icon: ScrollText,
    title: "Logs and tasks",
    text: "The server's recent log, and long jobs like purges and backups as they run.",
  },
  {
    icon: ServerCog,
    title: "Services",
    text: "Every service the server runs, with its worker's state and last error.",
  },
  {
    icon: ArrowDownUp,
    title: "Sort and filter",
    text: "Sort any table by any column, filter rows as you type, open tabs side by side.",
  },
];

export default function Home() {
  return (
    <>
      <SiteHeader />
      <main className="relative isolate">
        <Haze />

        {/* Hero */}
        <section className="mx-auto max-w-7xl px-5 pt-20 pb-12 text-center sm:pt-28">
          <Link
            href="/docs/status/"
            className="inline-flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.03] px-4 py-1.5 text-xs text-mist-300 transition hover:border-white/25 hover:text-white"
          >
            <span className="size-1.5 rounded-full bg-layer-peach" />
            Early and evolving · read the status before you deploy
            <ArrowRight className="size-3" />
          </Link>
          <h1 className="mx-auto mt-8 max-w-4xl text-5xl font-semibold tracking-tight text-white sm:text-7xl">
            Your Matrix server, <span className="text-layers">from the terminal.</span>
          </h1>
          <p className="mx-auto mt-6 max-w-2xl text-lg leading-relaxed text-mist-300 sm:text-xl">
            phantom is a Matrix homeserver written in Rust, with an admin console written in Go. Chat, manage
            accounts and rooms, read the logs and handle abuse reports, without leaving the keyboard.
          </p>
          <div className="mt-10 flex flex-col items-center gap-4">
            <CopyCommand command={site.install} />
            <div className="flex flex-wrap justify-center gap-3">
              <Link
                href="/docs/installation/"
                className="inline-flex items-center gap-2 rounded-xl bg-white px-5 py-2.5 text-sm font-semibold text-ink-900 transition hover:bg-mist-100"
              >
                Get started <ArrowRight className="size-4" />
              </Link>
              <a
                href={site.repo}
                className="inline-flex items-center gap-2 rounded-xl border border-white/10 px-5 py-2.5 text-sm font-semibold text-white transition hover:border-white/30"
              >
                <GithubIcon /> Star on GitHub
              </a>
            </div>
          </div>
        </section>

        <div className="relative mx-auto max-w-6xl px-5">
          <div
            aria-hidden
            className="absolute inset-x-16 top-10 -z-10 h-[80%] rounded-full bg-gradient-to-r from-layer-pink/25 via-layer-violet/20 to-layer-sky/25 blur-3xl"
          />
          <Media
            src="overview.gif"
            alt="Signing in to the phantom console, then walking its sections"
            priority
          />
        </div>

        {/* Showcase */}
        <Section
          id="features"
          eyebrow="Features"
          title={
            <>
              A homeserver and its console, <span className="text-layers">built together</span>.
            </>
          }
          intro="The server speaks the Matrix client-server API and an admin API of its own. The console uses both: any account can chat, and an admin sees and runs the whole server."
        >
          <div className="grid gap-5 lg:grid-cols-2">
            <Showcase
              icon={MessagesSquare}
              title="Chat in the terminal"
              text="Your rooms over /sync: messages, edits, reactions, typing and read receipts. Join and leave with a command."
              media="chat.gif"
              alt="Chatting in a room from the phantom console"
            />
            <Showcase
              icon={Wrench}
              title="Act on what you see"
              text="Enter on a row offers what can be done to it. Destructive actions ask first; every outcome is reported."
              media="actions.gif"
              alt="Making a user an admin and creating a registration token"
            />
            <Showcase
              icon={Activity}
              title="Run the server"
              text="Dismiss abuse reports, sort the media store, follow purges and backups as tasks."
              media="operations.gif"
              alt="Dismissing a report, sorting media and following tasks"
            />
            <Showcase
              icon={ServerCog}
              title="See inside it"
              text="Every service the server runs, whether its worker is up, and what it last failed with."
              media="services.png"
              alt="The services section of the phantom console"
            />
          </div>

          <div className="mt-5 grid gap-px overflow-hidden rounded-3xl border border-white/[0.07] bg-white/[0.07] sm:grid-cols-2 lg:grid-cols-4">
            {capabilities.map(({ icon: Icon, title, text }) => (
              <div key={title} className="bg-ink-850 p-7">
                <Icon className="size-5 text-layer-pink" />
                <h3 className="mt-4 font-semibold text-white">{title}</h3>
                <p className="mt-1.5 text-sm leading-relaxed text-mist-300">{text}</p>
              </div>
            ))}
          </div>
        </Section>

        {/* Sections */}
        <Section
          id="sections"
          eyebrow="Sections"
          title="Fifteen sections, one console."
          intro="Sign in as an admin and every section fills from the server. The first account registered on a fresh server is its admin."
        >
          <ScreenGallery />
        </Section>

        {/* Admin API */}
        <Section
          eyebrow="Under the hood"
          title="An admin API you can script too."
          intro="The console is a client of /_phantom/admin/v1, a JSON API behind an admin's access token. Anything it shows or does, curl can."
        >
          <div className="grid items-center gap-10 lg:grid-cols-2">
            <pre className="overflow-x-auto rounded-3xl border border-white/[0.07] bg-ink-950 p-7 font-mono text-[0.85rem] leading-7 text-mist-300">
              <code>
                <span className="text-mist-400"># what the overview shows</span>
                {"\n"}
                <span className="text-layer-pink">curl</span> -H{" "}
                <span className="text-layer-teal">&quot;Authorization: Bearer $TOKEN&quot;</span> \{"\n"}
                {"  "}localhost:8008/_phantom/admin/v1/stats{"\n\n"}
                <span className="text-mist-400"># create a registration token</span>
                {"\n"}
                <span className="text-layer-pink">curl</span> -X POST -H{" "}
                <span className="text-layer-teal">&quot;Authorization: Bearer $TOKEN&quot;</span> \{"\n"}
                {"  "}-d <span className="text-layer-teal">&apos;{"{"}&quot;uses_allowed&quot;: 10{"}"}&apos;</span> \{"\n"}
                {"  "}localhost:8008/_phantom/admin/v1/registration_tokens
              </code>
            </pre>
            <ul className="space-y-5 text-mist-300">
              {[
                ["Read everything", "Users, devices, rooms, tokens, services, federation, media, logs, reports, settings and tasks."],
                ["Change what matters", "Accounts, admin rights, devices, tokens, rooms, media, reports, the config and backups."],
                ["Secrets stay secret", "Settings are masked, and appservice and config-file tokens are never returned."],
              ].map(([title, text]) => (
                <li key={title} className="flex gap-4">
                  <span className="mt-1.5 size-2 shrink-0 rounded-full bg-gradient-to-br from-layer-pink to-layer-sky" />
                  <span>
                    <span className="font-semibold text-white">{title}.</span> {text}
                  </span>
                </li>
              ))}
              <li>
                <Link href="/docs/admin-api/" className="inline-flex items-center gap-2 font-semibold text-layer-sky hover:underline">
                  Read the admin API <ArrowRight className="size-4" />
                </Link>
              </li>
            </ul>
          </div>
        </Section>

        {/* CTA */}
        <section className="mx-auto max-w-7xl px-5 pb-28">
          <div className="relative overflow-hidden rounded-3xl border border-white/[0.07] bg-ink-850 px-8 py-16 text-center">
            <div
              aria-hidden
              className="absolute inset-0 -z-0 bg-gradient-to-br from-layer-pink/10 via-transparent to-layer-sky/10"
            />
            <h2 className="relative text-3xl font-semibold tracking-tight text-white sm:text-4xl">
              Build it, start it, sign in.
            </h2>
            <p className="relative mx-auto mt-4 max-w-xl text-mist-300">
              One build gives you the server and the console. Point the console at the server, register the first
              account, and you are its admin.
            </p>
            <div className="relative mt-8 flex justify-center">
              <CopyCommand command={site.install} />
            </div>
          </div>
        </section>
      </main>
      <SiteFooter />
    </>
  );
}
