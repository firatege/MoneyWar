// Yatay taşmanın kaynağını bul: viewport'u aşan elemanları adıyla listeler.
//
// audit-layout "sayfa yatay kayıyor" dediğinde suçluyu bu bulur — taşmayı
// gözle aramak yerine hangi kutunun kaç piksel aştığını söyler.
//
// Kullanım:  node scripts/find-overflow.mjs
import { chromium } from "@playwright/test";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
await page.goto(process.env.BASE ?? "http://localhost:8080", { waitUntil: "networkidle" });
await page.keyboard.press("Escape").catch(() => {});
await page.waitForTimeout(4000);

const out = await page.evaluate(() => {
  const w = document.documentElement.clientWidth;
  const bad = [];
  for (const el of document.querySelectorAll("*")) {
    const r = el.getBoundingClientRect();
    if (r.width === 0) continue;
    if (r.right > w + 1 || r.left < -1) {
      bad.push({
        sel: el.tagName + "." + String(el.className).split(" ").filter(Boolean).join("."),
        left: Math.round(r.left),
        right: Math.round(r.right),
        w: Math.round(r.width),
      });
    }
  }
  return { viewport: w, bad: bad.slice(0, 20) };
});
console.log(JSON.stringify(out, null, 2));
await browser.close();
