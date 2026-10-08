// The page's signs: one stroke, one weight, drawn in the colour of the
// words beside them.

import type { ReactNode } from "react";

function Sign({ size = 16, children }: { size?: number; children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      style={{ flex: "none" }}
    >
      {children}
    </svg>
  );
}

type Sized = { size?: number };

export const Plus = (props: Sized) => <Sign {...props}><path d="M12 5v14M5 12h14" /></Sign>;
export const ChatSign = (props: Sized) => <Sign {...props}><path d="M4 5h16v11H9l-5 4z" /></Sign>;
export const People = (props: Sized) => (
  <Sign {...props}>
    <circle cx="9" cy="9" r="3" />
    <path d="M3 19c0-3 3-5 6-5s6 2 6 5" />
    <circle cx="17" cy="8" r="2.5" />
    <path d="M16 14c3 0 5 2 5 4.5" />
  </Sign>
);
export const Folder = (props: Sized) => <Sign {...props}><path d="M3 6h7l2 2h9v11H3z" /></Sign>;
export const Prompt = (props: Sized) => (
  <Sign {...props}>
    <path d="M4 5h16v14H4z" />
    <path d="M8 10l3 2-3 2M13 15h3" />
  </Sign>
);
export const Clock = (props: Sized) => (
  <Sign {...props}>
    <circle cx="12" cy="12" r="8" />
    <path d="M12 8v4l3 2" />
  </Sign>
);
export const Laptop = (props: Sized) => (
  <Sign {...props}>
    <path d="M5 6h14v9H5z" />
    <path d="M3 18h18" />
  </Sign>
);
export const Gear = (props: Sized) => (
  <Sign {...props}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1" />
  </Sign>
);
export const Tiles = (props: Sized) => (
  <Sign {...props}>
    <rect x="4" y="4" width="7" height="7" rx="1.5" />
    <rect x="13" y="4" width="7" height="7" rx="1.5" />
    <rect x="4" y="13" width="7" height="7" rx="1.5" />
    <rect x="13" y="13" width="7" height="7" rx="1.5" />
  </Sign>
);
export const Shop = (props: Sized) => <Sign {...props}><path d="M4 9l1.5-4h13L20 9M5 9v10h14V9M9 19v-5h6v5" /></Sign>;
export const Sun = (props: Sized) => (
  <Sign {...props}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M18.4 5.6L17 7M7 17l-1.4 1.4" />
  </Sign>
);
export const Moon = (props: Sized) => <Sign {...props}><path d="M19 14a7.5 7.5 0 01-9-9 7.5 7.5 0 109 9z" /></Sign>;
export const Square = (props: Sized) => <Sign {...props}><rect x="7" y="7" width="10" height="10" rx="1.5" /></Sign>;
export const Clip = (props: Sized) => <Sign {...props}><path d="M8 12l6-6a3 3 0 014 4l-8 8a5 5 0 01-7-7l7-7" /></Sign>;
export const Wrench = (props: Sized) => <Sign {...props}><path d="M14 6a4 4 0 00-5 5l-5 5 3 3 5-5a4 4 0 005-5l-2.5 2.5-2-2z" /></Sign>;
export const Chevron = (props: Sized) => <Sign {...props}><path d="M9 6l6 6-6 6" /></Sign>;
export const Check = (props: Sized) => <Sign {...props}><path d="M5 12l5 5 9-10" /></Sign>;
export const Cross = (props: Sized) => <Sign {...props}><path d="M6 6l12 12M18 6L6 18" /></Sign>;
export const Arrow = (props: Sized) => <Sign {...props}><path d="M12 19V5M6 11l6-6 6 6" /></Sign>;
export const Copy = (props: Sized) => (
  <Sign {...props}>
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M5 15V5a1 1 0 011-1h10" />
  </Sign>
);
export const Down = (props: Sized) => <Sign {...props}><path d="M6 9l6 6 6-6" /></Sign>;
export const Key = (props: Sized) => (
  <Sign {...props}>
    <circle cx="8" cy="15" r="4" />
    <path d="M11 12l9-9M16 7l3 3" />
  </Sign>
);
export const Brain = (props: Sized) => (
  <Sign {...props}>
    <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z" />
    <path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" />
  </Sign>
);
export const Plug = (props: Sized) => (
  <Sign {...props}>
    <path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 01-12 0zM12 17v4" />
  </Sign>
);
export const Box = (props: Sized) => (
  <Sign {...props}>
    <path d="M4 8l8-4 8 4v8l-8 4-8-4z" />
    <path d="M4 8l8 4 8-4M12 12v8" />
  </Sign>
);
export const Again = (props: Sized) => <Sign {...props}><path d="M20 12a8 8 0 11-2.6-5.9M20 4v5h-5" /></Sign>;
export const Lock = (props: Sized) => (
  <Sign {...props}>
    <rect x="5" y="11" width="14" height="9" rx="2" />
    <path d="M8 11V8a4 4 0 018 0v3" />
  </Sign>
);
export const Phone = (props: Sized) => (
  <Sign {...props}>
    <rect x="7" y="3" width="10" height="18" rx="2" />
    <path d="M11 18h2" />
  </Sign>
);
export const Globe = (props: Sized) => (
  <Sign {...props}>
    <circle cx="12" cy="12" r="8" />
    <path d="M4 12h16M12 4c2.5 2.5 2.5 13.5 0 16M12 4c-2.5 2.5-2.5 13.5 0 16" />
  </Sign>
);
export const Server = (props: Sized) => (
  <Sign {...props}>
    <rect x="3" y="4" width="18" height="6" rx="1.5" />
    <rect x="3" y="14" width="18" height="6" rx="1.5" />
    <path d="M7 7h.01M7 17h.01" />
  </Sign>
);
