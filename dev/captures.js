// Captures d'écran du README (docs/screenshots/), prises sur l'aperçu navigateur
// (dev/serve.js + dev/mock.js) : fenêtre principale, vue DDC/CI et panneau de la
// zone de notification, en français et en anglais, en clair et en sombre.
// Usage : node dev/captures.js
// Prérequis : Playwright (npm i -D playwright), et les polices Segoe UI et
// Segoe MDL2 Assets / Segoe Fluent Icons installées (déjà là sous Windows ;
// sous Linux, à installer pour éviter les carrés à la place des icônes).
"use strict";
const { chromium } = require("playwright");
const { fork } = require("child_process");
const path = require("path");
const fs = require("fs");

const PORT = 4519;
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = path.join(__dirname, "..", "docs", "screenshots");

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const server = fork(path.join(__dirname, "serve.js"), ["ui", String(PORT)], { stdio: "ignore" });
  await new Promise((r) => setTimeout(r, 500));
  const browser = await chromium.launch();
  try {
    for (const lang of ["fr", "en"]) {
      for (const theme of ["light", "dark"]) {
        const ctx = await browser.newContext({
          viewport: { width: 1120, height: 740 }, // taille de la fenêtre principale (src/views.rs)
          deviceScaleFactor: 2,
          colorScheme: theme,
          locale: lang === "fr" ? "fr-FR" : "en-US",
        });
        const page = await ctx.newPage();

        // Fenêtre principale, vue Disposition.
        await page.goto(`${BASE}/?s=dock&lang=${lang}`);
        await page.waitForSelector(".tile");
        await page.waitForSelector("#inspector .settings");
        await page.waitForTimeout(400);
        await page.screenshot({ path: path.join(OUT, `window-${lang}-${theme}.png`) });

        // Vue DDC/CI.
        await page.click('.nav-item[data-view="ddc"]');
        await page.waitForSelector("#ddc-screens .group-title");
        await page.waitForTimeout(600);
        await page.screenshot({ path: path.join(OUT, `ddc-${lang}-${theme}.png`) });

        // Panneau de la zone de notification, à la hauteur que lui donne flyout_fit.
        const fly = await ctx.newPage();
        await fly.setViewportSize({ width: 360, height: 900 }); // FLYOUT_W (src/views.rs)
        await fly.goto(`${BASE}/flyout.html?s=dock&lang=${lang}`);
        await fly.waitForSelector("#fly:not([hidden])");
        await fly.waitForTimeout(800); // valeurs DDC/CI simulées (150 ms) puis rendu
        const h = await fly.evaluate(() => Math.ceil(parseFloat(document.body.style.height) || document.querySelector("#fly").getBoundingClientRect().height));
        await fly.setViewportSize({ width: 360, height: h });
        await fly.waitForTimeout(300);
        await fly.screenshot({ path: path.join(OUT, `flyout-${lang}-${theme}.png`), omitBackground: true });

        await ctx.close();
      }
    }
  } finally {
    await browser.close();
    server.kill();
  }
  console.log(`Captures écrites dans ${OUT}`);
})();
