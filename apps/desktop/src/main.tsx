import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "@fontsource/montserrat/800.css";
import "./theme.css";
import App from "./App";
import { initGateway } from "./api";

// Find out where the gateway is first (this PC, or the tunnel to a server).
void initGateway()
  .catch(() => undefined)
  .then(() =>
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <App />
      </StrictMode>,
    ),
  );
