import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { DashboardPage } from "./pages/DashboardPage.tsx";
import { AnalyticsPage } from "./pages/AnalyticsPage.tsx";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root bulunamadı");

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<DashboardPage />} />
        <Route path="/analytics" element={<AnalyticsPage />} />
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
