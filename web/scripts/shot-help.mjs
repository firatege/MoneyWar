// Yardım katmanının ekran görüntüsü — açılışta otomatik geliyor.
import { chromium } from "@playwright/test";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 950 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

await page.goto(process.env.BASE ?? "http://localhost:8080", { waitUntil: "networkidle" });
await page.waitForTimeout(2500);

const panel = page.locator(".help__panel");
if (!(await panel.count())) {
  // Daha önce kapatılmışsa düğmeden aç.
  await page.locator("button", { hasText: "nasıl çalışır" }).first().click();
  await page.waitForTimeout(600);
}
await page.locator(".help__panel").screenshot({ path: `${process.env.OUT ?? "/tmp"}/help.png` });
console.log(JSON.stringify({ errors }, null, 2));
await browser.close();
