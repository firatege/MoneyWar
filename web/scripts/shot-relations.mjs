// İlişki ağı sayfasının ekran görüntüsü + düzen denetimi.
import { chromium } from "@playwright/test";

const BASE = process.env.BASE ?? "http://localhost:8080";
const OUT = process.env.OUT ?? "/tmp";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const errors = [];
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
page.on("pageerror", (e) => errors.push(String(e)));

await page.goto(BASE, { waitUntil: "networkidle" });
await page.keyboard.press("Escape").catch(() => {});
// Ağın dolması için birkaç tick bekle.
await page.waitForTimeout(Number(process.env.WAIT ?? 20000));

await page.locator("button", { hasText: "ilişkiler" }).first().click();
await page.waitForTimeout(2500);
await page.screenshot({ path: `${OUT}/rel-1-overview.png` });

// Bir firmaya odaklan.
// Tıklama hedefi görünmez daire; `<g>`nin sınırlayıcı kutusu
// döndürülmüş etiketi de kapsadığı için merkezi boşluğa düşüyor.
const node = page.locator(".rel__hit").first();
if (await node.count()) {
  await node.click();
  await page.waitForTimeout(900);
  await page.screenshot({ path: `${OUT}/rel-2-focus.png` });
}

const problems = await page.evaluate(() => {
  const out = [];
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) out.push(`yatay taşma ${de.scrollWidth}>${de.clientWidth}`);
  for (const el of document.querySelectorAll(".rel__layout *")) {
    const cs = getComputedStyle(el);
    const clipped = el.scrollWidth > el.clientWidth + 2 || el.scrollHeight > el.clientHeight + 2;
    const scrollable = /auto|scroll/.test(cs.overflowX + cs.overflowY);
    if (clipped && !scrollable && cs.textOverflow !== "ellipsis" && el.clientHeight > 0) {
      const t = (el.textContent ?? "").trim().slice(0, 40);
      if (t) out.push(`kırpılmış: ${el.className} "${t}"`);
    }
  }
  return out.slice(0, 10);
});

console.log(JSON.stringify({ problems, errors: errors.slice(0, 8) }, null, 2));
await browser.close();
