/** A ghost, for phantom: a rounded head and a wavy hem, with two eyes. */
export function LogoMark({ className = "size-7" }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden>
      <defs>
        <linearGradient id="phantom-mark" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#f5c2e7" />
          <stop offset=".5" stopColor="#cba6f7" />
          <stop offset="1" stopColor="#89b4fa" />
        </linearGradient>
      </defs>
      <path
        d="M16 3.5c-6.4 0-10.5 4.6-10.5 10.6V27c0 .9 1 1.3 1.6.7l2.4-2.3 2.6 2.5c.5.5 1.2.5 1.7 0L16 25.4l2.2 2.5c.5.5 1.2.5 1.7 0l2.6-2.5 2.4 2.3c.6.6 1.6.2 1.6-.7V14.1C26.5 8.1 22.4 3.5 16 3.5Z"
        fill="url(#phantom-mark)"
      />
      <ellipse cx="12.4" cy="14.2" rx="1.8" ry="2.4" fill="#11111b" />
      <ellipse cx="19.6" cy="14.2" rx="1.8" ry="2.4" fill="#11111b" />
    </svg>
  );
}

export function Logo() {
  return (
    <span className="flex items-center gap-2.5">
      <LogoMark />
      <span className="font-mono text-lg font-semibold tracking-tight">phantom</span>
    </span>
  );
}
