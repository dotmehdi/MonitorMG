// Fonctions pures de l'interface, chargées comme dans la page (scripts classiques).
// Lancer : npm run test:ui
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

function page(lang = "fr") {
  // Le strict nécessaire du DOM pour que i18n.js parcoure une page vide.
  const document = { documentElement: {}, title: "", body: {}, createTreeWalker: () => ({ nextNode: () => null }), querySelectorAll: () => [] };
  const ctx = vm.createContext({ window: { LANG: lang }, document, NodeFilter: { SHOW_TEXT: 4 }, navigator: { language: "fr-FR" }, console });
  for (const f of ["i18n.js", "common.js", "vcp.js"]) vm.runInContext(readFileSync(new URL(`../ui/${f}`, import.meta.url), "utf8"), ctx, { filename: f });
  return (expr) => vm.runInContext(expr, ctx);
}

test("une tuile lâchée près d'un écran s'aimante contre son bord, sans chevauchement", () => {
  const ui = page();
  const snap = ui("snap");
  const main = { x: 0, y: 0, w: 3840, h: 2160 };
  // Lâchée un peu à droite et un peu plus bas que le haut de l'écran principal.
  const to = snap({ x: 3900, y: 60, w: 1920, h: 1080 }, [main]);
  assert.deepEqual({ x: to.x, y: to.y }, { x: 3840, y: 0 });
  // Lâchée par-dessus : ressort sur le bord le plus proche.
  const out = snap({ x: 1000, y: 1500, w: 1920, h: 1080 }, [main]);
  assert.ok(!ui("overlaps")({ ...out, w: 1920, h: 1080 }, main));
});

test("envoi au fil du glissement : une requête à la fois, la dernière valeur gagne", async () => {
  const latest = page()("latest");
  const sent = [];
  let release;
  const send = latest(
    (v) => {
      sent.push(v);
      return new Promise((r) => (release = r));
    },
    () => {},
  );
  send(10);
  send(20);
  send(30);
  send(40);
  release();
  await new Promise((r) => setTimeout(r, 0));
  release();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(sent, [10, 40]);
});

test("noms des valeurs DDC/CI", () => {
  const ui = page();
  const name = ui("vcpValueName");
  assert.equal(name(0x72, 0x78), "2,2");
  assert.equal(name(0x60, 0x11), "HDMI 1");
  assert.equal(name(0x60, 0x99), "0x99");
  assert.equal(ui("vcpName")(0xf0), "Réglage du fabricant 0xF0");
  assert.equal(page("en")("vcpName")(0x12), "Contrast");
});

test("traductions : texte français par défaut, arguments remplacés", () => {
  assert.equal(page("en")('t("Luminosité de {0}", "BenQ")'), "BenQ brightness");
  assert.equal(page("fr")('t("Luminosité de {0}", "BenQ")'), "Luminosité de BenQ");
});

test("heures des étapes : minutes depuis minuit <-> HH:MM", () => {
  const ui = page();
  assert.equal(ui("clock")(7 * 60 + 5), "07:05");
  assert.equal(ui("clock")(0), "00:00");
  assert.equal(ui("minutes")("22:30"), 22 * 60 + 30);
  assert.equal(ui("minutes")("7:05"), 7 * 60 + 5);
  for (const bad of ["", "24:00", "12:60", "midi", null]) assert.equal(ui("minutes")(bad), null);
});
