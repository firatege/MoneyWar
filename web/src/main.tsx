import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { DashboardPage } from "./pages/DashboardPage.tsx";
import { AnalyticsPage } from "./pages/AnalyticsPage.tsx";
import { FirmPage } from "./pages/FirmPage.tsx";
import { BucketPage } from "./pages/BucketPage.tsx";
import { MarketPage } from "./pages/MarketPage.tsx";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root bulunamadı");

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<DashboardPage />} />
        <Route path="/analytics" element={<AnalyticsPage />} />
        <Route path="/analytics/firm/:id" element={<FirmPage />} />
        <Route path="/analytics/bucket/:city/:product" element={<BucketPage />} />
        <Route path="/analytics/market" element={<MarketPage />} />
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
