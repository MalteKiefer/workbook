import React from "react";
import ReactDOM from "react-dom/client";
import QuickCapture from "./QuickCapture";
import "../styles/theme.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QuickCapture />
  </React.StrictMode>,
);
