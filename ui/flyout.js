"use strict";
// Panneau ouvert d'un clic sur l'icône de notification : luminosité de chaque écran,
// réglages DDC/CI choisis dans la fenêtre principale, profils, extinction des écrans.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const win = window.__TAURI__.window.getCurrentWindow();
const NNBSP = LANG === "fr" ? " " : "";
const $ = (s) => document.querySelector(s);

let S = null;
let holding = false; // curseur tenu : pas de reconstruction sous le pointeur

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

const glyph = (g) => el("span", { class: "i", "aria-hidden": "true", text: g });
const say = (text) => ($("#msg").textContent = text ?? "");
const close = () => win.close();

/** Envoi au fil du glissement : DDC/CI est lent, une requête à la fois, la dernière valeur gagne. */
function queue(send) {
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
        say(String(e));
      }
    }
    inflight = false;
  };
}

/** Ligne à curseur : icône, curseur, valeur. `value` null : en attente de l'écran. */
function sliderRow({ id, label, icon, value, onChange }) {
  const out = el("output", { class: "level-value", for: id });
  const input = el("input", { id, type: "range", min: 0, max: 100, step: 1, value: value ?? 0, "aria-label": label, disabled: value == null });
  const row = el("div", { class: "fly-feature", title: label }, el("span", { class: "fly-icon i", "aria-hidden": "true", text: icon }), el("span", { class: "slider" }, input), out);
  const show = (v) => {
    row.style.setProperty("--v", v);
    out.textContent = `${v}${NNBSP}%`;
    input.setAttribute("aria-valuetext", out.textContent);
  };
  const send = queue(onChange);
  input.addEventListener("input", () => {
    show(input.valueAsNumber);
    send(input.valueAsNumber);
  });
  input.addEventListener("pointerdown", () => {
    holding = true;
    document.addEventListener("pointerup", () => (holding = false), { once: true });
  });
  // La molette règle le curseur survolé, comme dans le panneau de volume de Windows.
  input.addEventListener("wheel", (e) => {
    if (input.disabled) return;
    e.preventDefault();
    input.valueAsNumber = Math.min(100, Math.max(0, input.valueAsNumber + (e.deltaY < 0 ? 2 : -2)));
    input.dispatchEvent(new Event("input"));
  }, { passive: false });
  const set = (v) => {
    input.disabled = v == null;
    if (v == null) {
      out.textContent = t("Sans réponse");
      row.style.setProperty("--v", 0);
      return;
    }
    input.value = v;
    show(v);
  };
  if (value == null) out.textContent = "…";
  else show(value);
  return { node: row, set };
}

/** Réglage à choix (entrée, préréglage couleur…) : une puce par valeur annoncée par l'écran. */
function choiceRow(m, sc, code) {
  const name = `c-${m.key.replace(/[^\w-]/g, "")}-${code}`;
  const hint = el("p", { class: "fly-hint" });
  let current = null;
  let armed = null;
  const chips = vcpValues(sc, code).map((v) => {
    const input = el("input", { type: "radio", name, value: v, disabled: true });
    input.addEventListener("change", async () => {
      // Changer d'entrée fait quitter ce PC à l'écran : on confirme d'un second clic.
      if (code === VCP_INPUT && armed !== v) {
        armed = v;
        hint.textContent = t("L'écran affichera {0} et quittera ce PC. Cliquez à nouveau pour confirmer.", vcpValueName(code, v));
        set(current);
        return;
      }
      armed = null;
      hint.textContent = "";
      try {
        await invoke("ddc_set", { key: m.key, code, value: v });
        current = v;
      } catch (e) {
        say(String(e));
        set(current);
      }
    });
    return el("label", { class: "chip" }, input, el("span", { text: vcpValueName(code, v) }));
  });
  const set = (v) => {
    current = v;
    for (const c of chips) {
      const i = c.querySelector("input");
      i.disabled = false;
      i.checked = Number(i.value) === v;
    }
  };
  const node = el(
    "div",
    { class: "fly-choice" },
    el("div", { class: "fly-choice-head" }, el("span", { class: "fly-icon i", "aria-hidden": "true", text: vcpIcon(code) }), el("span", { class: "fly-label", text: vcpName(code) })),
    el("div", { class: "fly-chips", role: "radiogroup", "aria-label": vcpName(code) }, ...chips),
    hint,
  );
  return { node, set };
}

const screenOf = (m) => S.screens[m.key] ?? {};
const nameOf = (m) => screenOf(m).name || m.name;
const shown = (m) => m.active && !m.mirror_of && !screenOf(m).hidden;

function monitorBlock(m, withBrightness) {
  const sc = screenOf(m);
  const feats = (sc.features ?? []).filter((f) => !f.linked);
  const power = feats.some((f) => f.code === VCP_POWER);
  const controls = new Map();
  const id = (code) => `f-${m.key.replace(/[^\w-]/g, "")}-${code}`;

  const head = el("div", { class: "fly-mon-head" }, el("h2", { class: "fly-name", text: nameOf(m) }));
  let powerBtn = null;
  if (power) {
    powerBtn = el("button", { class: "icon-btn", title: t("Éteindre l'écran"), "aria-label": t("Éteindre {0}", nameOf(m)), disabled: true }, glyph(""));
    powerBtn.addEventListener("click", async () => {
      try {
        const on = await invoke("ddc_power", { key: m.key });
        controls.get(VCP_POWER)?.(on ? 1 : 4);
      } catch (e) {
        say(String(e));
      }
    });
    controls.set(VCP_POWER, (v) => {
      const off = v != null && v !== 1;
      powerBtn.disabled = v == null;
      powerBtn.setAttribute("aria-pressed", String(off));
      powerBtn.title = off ? t("Allumer l'écran") : t("Éteindre l'écran");
      powerBtn.setAttribute("aria-label", t(off ? "Allumer {0}" : "Éteindre {0}", nameOf(m)));
    });
    head.append(powerBtn);
  }

  const rows = [];
  if (withBrightness && m.brightness != null) {
    rows.push(sliderRow({ id: id(0x10), label: t("Luminosité de {0}", nameOf(m)), icon: "", value: m.brightness, onChange: (pct) => invoke("set_brightness", { key: m.key, pct }) }).node);
  }
  for (const f of feats) {
    if (f.code === VCP_POWER) continue;
    const c = vcpDiscrete(sc, f.code)
      ? choiceRow(m, sc, f.code)
      : sliderRow({ id: id(f.code), label: t("{0} de {1}", vcpName(f.code), nameOf(m)), icon: vcpIcon(f.code), value: null, onChange: (value) => invoke("ddc_set", { key: m.key, code: f.code, value }) });
    controls.set(f.code, c.set);
    rows.push(c.node);
  }
  if (!rows.length && !power) return null;
  const block = el("section", { class: "fly-mon" }, head, ...rows);
  if (feats.length) {
    invoke("ddc_values", { key: m.key })
      .then((vals) => {
        for (const { code, value } of vals) controls.get(code)?.(value);
      })
      .catch((e) => {
        for (const set of controls.values()) set(null);
        say(String(e));
      });
  }
  return block;
}

function render() {
  const list = $("#list");
  const mons = S.monitors.filter(shown);
  const dimmable = mons.filter((m) => m.brightness != null);
  const linked = S.panel.linked && dimmable.length > 1;
  const link = $("#link");
  link.hidden = dimmable.length < 2;
  link.setAttribute("aria-pressed", String(linked));

  const blocks = [];
  if (linked) {
    const send = (pct) => dimmable.reduce((p, m) => p.then(() => invoke("set_brightness", { key: m.key, pct })), Promise.resolve());
    const all = sliderRow({ id: "all", label: t("Luminosité de tous les écrans"), icon: "", value: dimmable[0].brightness, onChange: send });
    blocks.push(el("section", { class: "fly-mon" }, el("div", { class: "fly-mon-head" }, el("h2", { class: "fly-name", text: t("Tous les écrans") })), all.node));
  }
  blocks.push(...mons.map((m) => monitorBlock(m, !linked)).filter(Boolean));
  if (!blocks.length) {
    blocks.push(
      el(
        "div",
        { class: "fly-empty" },
        el("p", { text: t("Aucun écran ne répond en DDC/CI. Activez DDC/CI dans le menu de l'écran, puis choisissez ses réglages.") }),
        el("button", { class: "btn", onclick: () => invoke("open_main", { view: "ddc" }) }, t("Réglages DDC/CI")),
      ),
    );
  }
  list.replaceChildren(...blocks);

  const profiles = S.profiles;
  $("#foot").hidden = !profiles.length;
  $("#profiles").replaceChildren(
    ...profiles.map((p) =>
      el("button", {
        class: "fly-profile",
        "aria-pressed": String(p.active),
        text: p.name,
        onclick: async () => {
          say(t("Application en cours…"));
          try {
            const warn = await invoke("profile_apply", { id: p.id, panel: true });
            say(warn || "");
            load();
          } catch (e) {
            say(String(e));
          }
        },
      }),
    ),
  );
  fit();
}

/** La fenêtre prend la hauteur du panneau, sans dépasser l'écran. */
function fit() {
  const fly = $("#fly");
  fly.hidden = false;
  fly.style.maxHeight = "none";
  const h = fly.offsetHeight + 24; // marge transparente de la page, 12 px de chaque côté
  fly.style.maxHeight = "";
  invoke("flyout_fit", { h }).catch(() => {});
}

async function load() {
  if (holding) return setTimeout(load, 300);
  try {
    S = await invoke("state");
  } catch (e) {
    say(String(e));
    return;
  }
  const root = document.documentElement.style;
  root.setProperty("--accent-d", S.accent[0]);
  root.setProperty("--accent-l", S.accent[1]);
  render();
}

$("#link").addEventListener("click", async () => {
  S.panel.linked = !S.panel.linked;
  render();
  try {
    await invoke("set_panel", { panel: S.panel });
  } catch (e) {
    say(String(e));
  }
});
$("#off").addEventListener("click", () => invoke("screens_off_now").finally(close));
$("#settings").addEventListener("click", () => invoke("open_main", { view: "ddc" }));
document.addEventListener("keydown", (e) => e.key === "Escape" && close());
document.addEventListener("contextmenu", (e) => e.preventDefault());
listen("changed", () => load());
load();
