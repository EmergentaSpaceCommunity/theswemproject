import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

import { Guard } from "./Guard.tsx";
import { Door } from "./page/Door.tsx";
import { Workbench } from "./page/Workbench.tsx";

declare global {
  interface Window {
    __SWEM_WORKBENCH_WEB__?: string;
  }
}

const container = document.getElementById("workbench-root");
if (!container) throw new Error("Workbench mount is missing");

flushSync(() =>
  createRoot(container).render(
    <StrictMode>
      <Guard what="The Workbench">
        <Door>
          <Workbench />
        </Door>
      </Guard>
    </StrictMode>,
  ),
);
container.dataset.renderer = "react";
window.__SWEM_WORKBENCH_WEB__ = "react-19";
