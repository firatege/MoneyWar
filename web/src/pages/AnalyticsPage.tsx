// Analitik sayfası — firmalar, piyasa derinliği, ilişki grafiği.
// Şu an iskelet; veri bağlantısı kademeli doldurulacak.

import { Link } from "react-router-dom";
import "./pages.css";

export function AnalyticsPage() {
  return (
    <div className="page-analytics">
      <header className="page-analytics__head">
        <Link to="/" className="page-analytics__back">← Dashboard</Link>
        <h1 className="page-analytics__title">ANALİTİK</h1>
      </header>
      <div className="page-analytics__body">
        <p className="page-analytics__placeholder">
          Yakında: firma rekabet haritası, ilişki ağı, fiyat geçmişi derinliği.
        </p>
      </div>
    </div>
  );
}
