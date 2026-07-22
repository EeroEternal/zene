import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement>;

export const IconRepo = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M4 4h12a2 2 0 0 1 2 2v14l-4-2-4 2-4-2-4 2V6a2 2 0 0 1 2-2z" />
    <path d="M8 8h6M8 12h6" />
  </svg>
);

export const IconBranch = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <circle cx="6" cy="6" r="2" />
    <circle cx="6" cy="18" r="2" />
    <circle cx="18" cy="12" r="2" />
    <path d="M6 8v8M8 18h8a2 2 0 0 0 2-2v-4" />
  </svg>
);

export const IconRefresh = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5" />
  </svg>
);

export const IconGithub = (p: P) => (
  <svg viewBox="0 0 16 16" fill="currentColor" {...p}>
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.53-.49-.45-1.16-1.11-1.48-1.11-1.48-.91-.62-.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.87 1.52 2.34 1.07 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.92 0-1.11.38-2 1.03-2.71-.1-.25-.45-1.29.1-2.64 0 0 .84-.27 2.75 1.02.79-.22 1.65-.33 2.5-.33.85 0 1.71.11 2.5.33 1.91-1.29 2.75-1.02 2.75-1.02.55 1.35.2 2.39.1 2.64.65.71 1.03 1.6 1.03 2.71 0 3.82-2.34 4.66-4.57 4.91.36.31.69.92.69 1.85V16c0 .27.18.58.67.49C13.71 14.53 16 11.54 16 8 16 3.58 12.42 0 8 0z" />
  </svg>
);

export const IconGitlab = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M12 21 2.5 14.5l2-6.5L7 14h10l2.5-6 2 6.5L12 21zM7 14 9.5 3l2.5 7h0l2.5-7L17 14" />
  </svg>
);

export const IconCheck = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="m5 12 5 5L20 7" />
  </svg>
);

export const IconExternal = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="M7 17 17 7M9 7h8v8" />
  </svg>
);

export const IconPlug = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M9 7v4H7a2 2 0 0 0 0 4h2v2a2 2 0 0 0 4 0v-2h2a2 2 0 0 0 0-4h-2V7a2 2 0 0 0-4 0z" />
    <path d="M15 9h3M6 15H3" />
  </svg>
);

export const IconChevronDown = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="m6 9 6 6 6-6" />
  </svg>
);

export const IconChevronRight = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="m9 6 6 6-6 6" />
  </svg>
);

export const IconSearch = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </svg>
);

export const IconPlus = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const IconDots = (p: P) => (
  <svg viewBox="0 0 24 24" fill="currentColor" {...p}>
    <circle cx="5" cy="12" r="2" />
    <circle cx="12" cy="12" r="2" />
    <circle cx="19" cy="12" r="2" />
  </svg>
);

export const IconFilter = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M4 7h16M4 12h10M4 17h7" />
  </svg>
);

export const IconSettings = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);

export const IconHelp = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M9.5 9a2.5 2.5 0 1 1 3.8 2.1c-.8.5-1.3 1-1.3 2v.4" />
    <circle cx="12" cy="17" r=".8" fill="currentColor" stroke="none" />
  </svg>
);

export const IconLogout = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M10 7V5a2 2 0 0 1 2-2h7v18h-7a2 2 0 0 1-2-2v-2" />
    <path d="M3 12h11M10 8l4 4-4 4" />
  </svg>
);

export const IconPaperclip = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="m21.4 11.6-8.8 8.8a5 5 0 0 1-7.1-7.1l8.8-8.8a3.2 3.2 0 0 1 4.5 4.5l-8.5 8.5a1.4 1.4 0 0 1-2-2l7.8-7.8" />
  </svg>
);

export const IconSkills = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke" {...p}>
    <path d="M4 5h7v14l-3.5-2L4 19V5zM13 5h7v14l-3.5-2L13 19V5z" />
  </svg>
);

export const IconArrowUp = (p: P) => (
  <svg viewBox="0 0 24 24" className="ico-stroke-2" {...p}>
    <path d="M12 19V5M5 12l7-7 7 7" />
  </svg>
);
