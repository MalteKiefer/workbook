import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/theme.css";
import { applyPersistedTheme, listenForThemeChanges } from "./lib/theme";

// Fire-and-forget: applies whenever the async read resolves rather than
// blocking first paint on it, accepting a possible one-frame flash of the
// default dark theme on cold start as an acceptable first-version tradeoff.
void applyPersistedTheme();
listenForThemeChanges();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
