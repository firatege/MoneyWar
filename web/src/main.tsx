import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { DashboardPage } from "./pages/DashboardPage.tsx";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root bulunamadı");

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        {/* Tek sayfa. Ayrı "analitik" bölümü vardı; içeriğinin tamamı
            dashboard'ın üç kademeli inceleme akışına taşındı — orada
            kalan son iki şey (fiyat seyri, firma PnL eğrisi) şehir ve
            firma sayfalarına eklendi. İki paralel arayüzü ayakta tutmak
            hem bakım yüküydü hem de "insan ne aradığını bulmalı"
            hedefine ters düşüyordu. */}
        <Route path="/" element={<DashboardPage />} />
        <Route path="*" element={<DashboardPage />} />
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
