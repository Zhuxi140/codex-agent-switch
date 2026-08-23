import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { ProjectMonitorWindow } from "./ProjectMonitorWindow";
import { ProjectMonitorWindowPreview } from "./ProjectMonitorWindow.preview";
import "./styles.css";

const windowType = new URLSearchParams(window.location.search).get("window");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {windowType === "project-monitor"
      ? <ProjectMonitorWindow />
      : windowType === "project-monitor-preview"
        ? <ProjectMonitorWindowPreview />
        : <App />}
  </StrictMode>,
);
