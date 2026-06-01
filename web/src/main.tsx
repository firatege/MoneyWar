import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root bulunamadı");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
