import React from "react";
import ReactDOM from "react-dom/client";
import { RecorderCompanion } from "./RecorderCompanion";
import "../App.css";
import "../styles/kx.css";

// Kōrero 1.42: the companion window (C1 on the design canvas). It owns no
// recording state — it draws what the main window broadcasts and sends
// commands back, so closing it never affects the meeting.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RecorderCompanion />
  </React.StrictMode>,
);
