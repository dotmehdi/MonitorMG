"use strict";
// Outils partagés par la fenêtre principale et le panneau de la zone de notification :
// création d'éléments, envoi « dernière valeur » vers DDC/CI, aimantation de la disposition.
// Les fonctions pures sont testées par `node --test tests/` (voir tests/ui.test.mjs).

const $ = (s) => document.querySelector(s);
const NNBSP = LANG === "fr" ? " " : ""; // espace fine insécable avant « % », en français seulement

/** Crée un élément : el("button", { class, text, onclick, ... }, ...enfants) */
function el(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? "" : v);
  }
  e.append(...kids.filter((k) => k != null && k !== false));
  return e;
}

/**
 * Envoi au fil d'un glissement : DDC/CI est lent, une requête à la fois, la dernière
 * valeur gagne. Après un échec, `onError` le reçoit et les valeurs en attente sont abandonnées.
 */
function latest(send, onError) {
  let inflight = false;
  let queued = null;
  return async (v) => {
    queued = v;
    if (inflight) return;
    inflight = true;
    while (queued !== null) {
      const x = queued;
      queued = null;
      try {
        await send(x);
      } catch (e) {
        queued = null;
        onError(e);
      }
    }
    inflight = false;
  };
}

/** Minutes depuis minuit -> « HH:MM », le format des champs d'heure. */
const clock = (min) => `${String(Math.floor(min / 60)).padStart(2, "0")}:${String(min % 60).padStart(2, "0")}`;

/** « HH:MM » -> minutes depuis minuit ; null si ce n'est pas une heure. */
function minutes(hhmm) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm ?? "");
  return m && Number(m[1]) < 24 && Number(m[2]) < 60 ? Number(m[1]) * 60 + Number(m[2]) : null;
}

const overlaps = (a, b) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

/**
 * Aimante un rectangle contre le bord le plus proche d'un autre écran, sans chevauchement,
 * comme dans les paramètres de Windows. Les bords proches s'alignent (haut, bas, centre).
 */
function snap(r, others) {
  let best = null;
  let bestD = Infinity;
  for (const o of others) {
    const tolY = Math.min(r.h, o.h) * 0.08;
    const tolX = Math.min(r.w, o.w) * 0.08;
    const alignY = (y) => {
      for (const t of [o.y, o.y + o.h - r.h, o.y + (o.h - r.h) / 2]) if (Math.abs(y - t) < tolY) return t;
      return clamp(y, o.y - r.h + 1, o.y + o.h - 1);
    };
    const alignX = (x) => {
      for (const t of [o.x, o.x + o.w - r.w, o.x + (o.w - r.w) / 2]) if (Math.abs(x - t) < tolX) return t;
      return clamp(x, o.x - r.w + 1, o.x + o.w - 1);
    };
    const cands = [
      { x: o.x - r.w, y: alignY(r.y) },
      { x: o.x + o.w, y: alignY(r.y) },
      { x: alignX(r.x), y: o.y - r.h },
      { x: alignX(r.x), y: o.y + o.h },
    ];
    for (const c of cands) {
      const rect = { ...c, w: r.w, h: r.h };
      if (others.some((q) => overlaps(rect, q))) continue;
      const d = Math.hypot(c.x - r.x, c.y - r.y);
      if (d < bestD) [best, bestD] = [c, d];
    }
  }
  return best ?? { x: r.x, y: r.y };
}
