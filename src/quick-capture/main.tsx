import React from "react";
import ReactDOM from "react-dom/client";
import QuickCapture from "./QuickCapture";
import "../styles/theme.css";
import { applyPersistedTheme, listenForThemeChanges } from "../lib/theme";

// Separate webview/document from the main window -- applies the theme
// preference independently here too, see src/main.tsx for the same call.
void applyPersistedTheme();
listenForThemeChanges();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QuickCapture />
  </React.StrictMode>,
);
