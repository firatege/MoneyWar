// Düzen denetimi — üç kademede ekran görüntüsü alır ve ölçülebilir
// sorunları arar: yatay taşma, kırpılan metin, konsol hataları.
//
// "Göze güzel geliyor mu" sorusunu insan cevaplar; bunun cevapladığı soru
// "kırpılan sayı, taşan kutu, patlayan konsol var mı" — ve bunlar
// ekran görüntüsüne bakarak kaçırılabilecek şeyler.
//
// Kullanım:  node scripts/audit-layout.mjs   (sunucu localhost:8080'de)
//            BASE=https://... OUT=/tmp/x node scripts/audit-layout.mjs
import { chromium } from "@playwright/test";

const BASE = process.env.BASE ?? "http://localhost:8080";
const OUT = process.env.OUT ?? "/tmp/mw-shots";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });

const errors = [];
page.on("console", (m) => {
  if (m.type() === "error") errors.push(m.text());
});
page.on("pageerror", (e) => errors.push(String(e)));

await page.goto(BASE, { waitUntil: "networkidle" });
// Tanıtım açılırsa kapat.
const close = page.locator("button", { hasText: /kapat|başla|anladım/i }).first();
if (await close.count()) await close.click().catch(() => {});
await page.keyboard.press("Escape").catch(() => {});
await page.waitForTimeout(9000); // birkaç tick veri birikinsin

async function audit(label) {
  return await page.evaluate(() => {
    const problems = [];
    const de = document.documentElement;
    if (de.scrollWidth > de.clientWidth + 1) {
      problems.push(`sayfa yatay kayıyor: ${de.scrollWidth} > ${de.clientWidth}`);
    }
    // Kırpılan metin: taşan ama ellipsis'i olmayan kutular.
    for (const el of document.querySelectorAll("*")) {
      const cs = getComputedStyle(el);
      if (cs.overflow === "visible" && cs.display !== "inline") continue;
      const clippedX = el.scrollWidth > el.clientWidth + 2;
      const clippedY = el.scrollHeight > el.clientHeight + 2;
      const scrollable =
        cs.overflowX === "auto" || cs.overflowX === "scroll" ||
        cs.overflowY === "auto" || cs.overflowY === "scroll";
      const hasEllipsis = cs.textOverflow === "ellipsis";
      if ((clippedX || clippedY) && !scrollable && !hasEllipsis && el.clientHeight > 0) {
        const t = (el.textContent ?? "").trim().slice(0, 40);
        if (t) problems.push(`kırpılmış: ${el.className || el.tagName} "${t}"`);
      }
    }
    return problems.slice(0, 15);
  });
}

const report = {};
report.dashboard = await audit();
await page.screenshot({ path: `${OUT}/01-dashboard.png` });

// Şehir kademesi
const node = page.locator(".netmap__node").first();
if (await node.count()) {
  await node.click();
  await page.waitForTimeout(1500);
  report.city = await audit();
  await page.screenshot({ path: `${OUT}/02-city.png` });

  // Firma kademesi
  const firm = page.locator(".dt__linkbtn").first();
  if (await firm.count()) {
    await firm.click();
    await page.waitForTimeout(1500);
    report.firm = await audit();
    await page.screenshot({ path: `${OUT}/03-firm.png` });

    // Fabrika kademesi
    const fac = page.locator(".dt__card").first();
    if (await fac.count()) {
      await fac.click();
      await page.waitForTimeout(1500);
      report.factory = await audit();
      await page.screenshot({ path: `${OUT}/04-factory.png` });
    } else {
      report.factory = ["fabrika kartı yok (firmanın fabrikası olmayabilir)"];
    }
  }
}

// Dar ekran
await page.setViewportSize({ width: 390, height: 844 });
await page.goto(BASE, { waitUntil: "networkidle" });
await page.keyboard.press("Escape").catch(() => {});
await page.waitForTimeout(4000);
report.mobile = await audit();
await page.screenshot({ path: `${OUT}/05-mobile.png`, fullPage: true });

console.log(JSON.stringify({ report, errors: errors.slice(0, 10) }, null, 2));
await browser.close();
