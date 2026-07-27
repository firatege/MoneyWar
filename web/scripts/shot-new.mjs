// Yeni iki yüzey: fiyat ızgarası sayfası ve firma pazar kontrolü bölümü.
// Taşma ve çakışma denetimi ekran görüntüsüyle birlikte yapılır — CSS'i
// gözle onaylamak yetmiyor, sayı gerek.
import { chromium } from "@playwright/test";

const BASE = process.env.BASE ?? "https://moneywar.byfeb.com";
const OUT = process.env.OUT ?? "/tmp";

const browser = await chromium.launch();
const errors = [];

for (const [w, h, tag] of [[1600, 950, "wide"], [860, 900, "narrow"]]) {
  const page = await browser.newPage({ viewport: { width: w, height: h } });
  page.on("console", (m) => m.type() === "error" && errors.push(`[${tag}] ${m.text()}`));
  page.on("pageerror", (e) => errors.push(`[${tag}] ${String(e)}`));

  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.keyboard.press("Escape").catch(() => {});
  await page.waitForTimeout(Number(process.env.WAIT ?? 12000));

  // 1) Izgara sayfası
  await page.locator("button", { hasText: "ızgara" }).first().click();
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/grid-${tag}.png`, fullPage: true });
  const gridOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );

  await page.keyboard.press("Escape");
  await page.waitForTimeout(800);

  // 2) Firma sayfası — pazar kontrolü
  await page.locator(".rank__row").first().click();
  await page.waitForTimeout(1800);
  const gripCount = await page.locator(".dt__grip-row").count();
  await page.locator(".dt__grip").first().scrollIntoViewIfNeeded().catch(() => {});
  await page.screenshot({ path: `${OUT}/firm-${tag}.png`, fullPage: true });
  const firmOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );

  console.log(
    `${tag}: ızgara yatay taşma ${gridOverflow}px · firma taşma ${firmOverflow}px · pazar satırı ${gripCount}`,
  );
  await page.close();
}

console.log(errors.length ? `HATA:\n${errors.join("\n")}` : "konsol temiz");
await browser.close();
