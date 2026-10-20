import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./index.css";
import { invoke } from "@tauri-apps/api/core";

// Forward uncaught UI errors to the backend log (visible in the Dashboard log panel).
const report = (message: string) => void invoke("ui_log", { level: "error", message }).catch(() => {});
window.addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled rejection: ${String(e.reason)}`));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
