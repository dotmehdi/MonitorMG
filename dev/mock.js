// Faux backend Tauri pour l'aperçu navigateur.
(() => {
  // Pas de Mica dans un navigateur : fond opaque, comme sous Windows 10.
  document.documentElement.classList.add("solid");
  const q = new URLSearchParams(location.search);
  // Le backend pose window.LANG selon la langue de Windows ; ici, ?lang=en.
  window.LANG = q.get("lang") === "en" ? "en" : "fr";
  const L = (fr, en) => (window.LANG === "en" ? en : fr);
  const modes = (list) => list.map(([w, h, rates]) => ({ w, h, rates }));
  const fourK = modes([
    [3840, 2160, [240, 200, 144, 120, 60, 50]],
    [3440, 1440, [144, 120, 60]],
    [2560, 1440, [240, 165, 144, 120, 60]],
    [1920, 1080, [240, 144, 120, 60, 50]],
    [1280, 720, [60, 50]],
  ]);
  const laptop = modes([[2560, 1600, [165, 60]], [1920, 1200, [165, 60]], [1680, 1050, [60]]]);
  const builtin = L("Écran intégré", "Built-in display");
  const mon = (o) => ({ mirror_of: null, rotation: 0, modes: [], scale: null, hdr: null, brightness: null, soft: false, x: 0, y: 0, hz: 0, internal: false, primary: false, ...o });
  const scenarios = {
    desk: [
      mon({ key: "a", name: "AORUS FO32U2", connector: "DisplayPort", active: true, primary: true, w: 3840, h: 2160, hz: 200, native: [3840, 2160], modes: fourK, scale: { current: 200, recommended: 150, options: [100, 125, 150, 175, 200, 225, 250, 300] }, hdr: false, brightness: 60 }),
      mon({ key: "b", name: "BenQ EW3270U", connector: "DisplayPort", active: false, w: 3840, h: 2160, native: [3840, 2160] }),
    ],
    dock: [
      mon({ key: "a", name: "DELL U2723QE", connector: "USB-C", active: true, primary: true, w: 3840, h: 2160, hz: 60, native: [3840, 2160], modes: fourK, scale: { current: 150, recommended: 150, options: [100, 125, 150, 175, 200, 225, 250, 300] }, hdr: false, brightness: 75 }),
      mon({ key: "b", name: builtin, connector: L("Intégré", "Built-in"), internal: true, active: true, x: -2560, y: 560, w: 2560, h: 1600, hz: 165, native: [2560, 1600], modes: laptop, scale: { current: 150, recommended: 150, options: [100, 125, 150, 175, 200] }, brightness: 40 }),
      mon({ key: "c", name: "LG 27UL850", connector: "HDMI", active: true, x: 3840, y: -400, w: 2160, h: 3840, hz: 60, rotation: 90, native: [3840, 2160], modes: fourK.map((m) => ({ ...m, w: m.h, h: m.w })), scale: { current: 150, recommended: 150, options: [100, 125, 150, 175, 200] }, hdr: true, brightness: 80, soft: true }),
      mon({ key: "d", name: "Samsung Odyssey G5", connector: "DisplayPort", active: false, w: 2560, h: 1440, native: [2560, 1440] }),
    ],
    // Dupliquer : même image, chaque écran garde sa luminosité et son HDR.
    clone: [
      mon({ key: "a", name: "AORUS FO32U2", connector: "DisplayPort", active: true, primary: true, w: 3840, h: 2160, hz: 60, native: [3840, 2160], modes: fourK, scale: { current: 200, recommended: 150, options: [100, 125, 150, 175, 200, 225, 250, 300] }, hdr: false, brightness: 60 }),
      mon({ key: "b", name: "BenQ EW3270U", connector: "DisplayPort", active: true, mirror_of: "a", w: 3840, h: 2160, hz: 60, native: [3840, 2160], modes: fourK, scale: { current: 200, recommended: 150, options: [100, 125, 150, 175, 200, 225, 250, 300] }, hdr: false, brightness: 45 }),
    ],
  };
  const sc = q.get("s") || "dock";
  const tiles = (ms) => ms.filter((m) => m.active).map(({ name, x, y, w, h, hz, primary }) => ({ name, x, y, w, h, hz, primary }));
  const state = {
    monitors: scenarios[sc],
    profiles: [
      { id: "1", name: L("Bureau", "Desk"), hotkey: "Ctrl+Alt+Digit1", tiles: tiles(scenarios.dock), active: sc === "dock", auto: true },
      { id: "2", name: L("Nomade", "Travel"), hotkey: "Ctrl+Alt+Digit2", tiles: [{ name: builtin, x: 0, y: 0, w: 2560, h: 1600, hz: 165, primary: true }], active: false, auto: false },
      { id: "3", name: L("Présentation salle B", "Room B presentation"), hotkey: null, tiles: [{ name: builtin, x: 0, y: 0, w: 1920, h: 1080, hz: 60, primary: true }], active: false, auto: false },
      { id: "4", name: L("Jeu 240 Hz", "Gaming 240 Hz"), hotkey: "Ctrl+Alt+Shift+KeyG", tiles: tiles(scenarios.desk), active: sc === "desk", auto: false },
    ],
    topology: sc === "clone" ? "clone" : "extend",
    autostart: true,
    confirm: { on: true, secs: 15 },
    identify: { lit: null, dim: 0 },
    accent: ["#76b9ed", "#005a9e"],
    undo: L("DELL U2723QE : 3840 × 2160 à 60 Hz", "DELL U2723QE: 3840 × 2160 at 60 Hz"),
    version: "1.0.0",
    portable: q.get("portable") === "1",
    data_dir: L(String.raw`C:\Users\vous\AppData\Roaming\MonitorMG`, String.raw`C:\Users\you\AppData\Roaming\MonitorMG`),
  };
  const now = Date.now();
  const [win, menu, hotkey, plug, sys] = [L("Fenêtre", "Window"), "Menu", L("Raccourci", "Shortcut"), L("Branchement", "Connection"), L("Système", "System")];
  const log = [
    { at: now - 60e3, level: "ok", source: win, text: state.undo, undo: true },
    { at: now - 5 * 60e3, level: "warn", source: sys, text: L("Affichage précédent rétabli (sans confirmation)", "Previous display settings restored (no confirmation)") },
    { at: now - 42 * 60e3, level: "ok", source: plug, text: L("Profil « Bureau » appliqué au branchement", "Profile “Desk” applied on connect") },
    { at: now - 43 * 60e3, level: "ok", source: sys, text: L("LG 27UL850 branché", "LG 27UL850 connected") },
    { at: now - 26 * 3600e3, level: "error", source: hotkey, text: L("Profil « Jeu 240 Hz » appliqué : échec. Aucun écran de ce profil n'est branché", "Profile “Gaming 240 Hz” applied: failed. None of this profile's displays are connected") },
    { at: now - 27 * 3600e3, level: "ok", source: menu, text: L("Projection : étendre", "Projection: extend") },
    { at: now - 27 * 3600e3, level: "info", source: sys, text: L("MonitorMG 1.0.0 démarré", "MonitorMG 1.0.0 started") },
  ];
  const pct = (v) => L(`${v} %`, `${v}%`);
  const extras = (scale, hdr, brightness, windows = 0) => [
    { id: "scale", on: true, stored: scale },
    { id: "hdr", on: true, stored: hdr },
    { id: "brightness", on: brightness !== "", stored: brightness },
    { id: "windows", on: false, stored: windows ? L(`${windows} fenêtres`, `${windows} windows`) : "" },
  ];
  state.profiles[0].extras = extras(`${pct(150)} / ${pct(150)} / ${pct(150)}`, L("désactivé / activé", "off / on"), `${pct(75)} / ${pct(40)}`, 9);
  state.updates = false;
  // ?update=2.3.0 : une version trouvée ; &step=ready, download ou install : où elle en est.
  state.update = q.get("update");
  state.update_step = state.update ? q.get("step") : null;
  state.auto_update = { download: false, install: false, silent: true, keep_data: true };
  state.profiles[1].extras = extras(pct(150), "", pct(60));
  state.profiles[2].extras = extras(pct(100), "", "");
  state.profiles[3].extras = extras(pct(200), L("activé", "on"), pct(60));
  state.taskbar_light = q.get("taskbar") === "light";
  state.tray_icon = "ecrans";
  state.off_hotkey = null;
  state.up_hotkey = null;
  state.down_hotkey = null;
  state.panel = { linked: false, power: 5, off: "soft", step: 10 };
  state.schedule = { on: true, smooth: true, steps: [[7 * 60 + 30, 70], [13 * 60, 85], [19 * 60, 55], [22 * 60 + 30, 25]] };
  const triggers = (o = {}) => ({ at: null, power: null, exe: "", ...o });
  state.profiles.forEach((p, i) => (p.triggers = triggers([{}, { power: "dc" }, {}, { exe: "jeu.exe" }][i])));
  // Capacités relevées sur un AORUS FO32U2 réel.
  const aorus = { 2: [], 4: [], 5: [], 6: [], 8: [], 11: [], 12: [], 16: [], 18: [], 20: [4, 5, 7, 8, 11], 22: [], 24: [], 26: [], 82: [], 96: [1, 3, 15, 17], 98: [], 135: [], 141: [1, 2], 172: [], 174: [], 178: [], 182: [], 198: [], 200: [], 202: [], 204: [1, 2, 3, 4, 6, 10, 13], 214: [1, 4, 5], 223: [], 253: [], 255: [] };
  const feat = (code, o = {}) => ({ code, range: [0, 100], linked: false, stop: 100, ...o });
  const screen = (o) => ({ name: "", hidden: false, blocked: false, vcp: null, range: [0, 100], features: [], caps: {}, inputs: null, input_hotkey: null, ...o });
  state.screens = {
    a: screen({ caps: aorus, features: [feat(0x12), feat(0x62), feat(0x60), feat(0x14), feat(0xd6)], inputs: [15, 17], input_hotkey: "Ctrl+Alt+KeyK" }),
    c: screen({ name: L("Vertical", "Vertical"), caps: { 16: [], 18: [], 98: [], 214: [1, 4, 5] }, features: [feat(0x12, { linked: true, stop: 60 })] }),
  };
  const ddc = { 0x12: 50, 0x62: 2, 0x60: 15, 0x14: 5, 0xd6: 1 };
  // Comme le backend : chaque écran branché a ses préférences, par défaut au besoin.
  for (const m of state.monitors) state.screens[m.key] ??= screen({});
  state.auto_vcp = Object.fromEntries(Object.keys(state.screens).map((k) => [k, 0x10]));
  if (q.get("empty")) state.profiles = [];
  const delay = (ms) => new Promise((r) => setTimeout(r, ms));
  window.__emit = (name, payload) => window.__events?.[name]?.({ payload });
  window.__TAURI__ = {
    core: {
      async invoke(cmd, args) {
        console.log("invoke", cmd, args);
        await delay({ state: 30, set_brightness: 50, ddc_set: 50, ddc_values: 150, flyout_fit: 0, ddc_caps: 1500 }[cmd] ?? 400); // DDC/CI : ~50 ms par écriture
        if (cmd === "state") return structuredClone(state);
        if (cmd === "activity") return structuredClone(log);
        if (cmd === "undo") {
          const e = { at: Date.now(), level: "ok", source: win, text: L(`Annulé : ${state.undo}`, `Undone: ${state.undo}`), undo: true };
          log.unshift(e);
          setTimeout(() => window.__emit("activity", e), 0);
        }
        if (cmd === "set_brightness") state.monitors.find((m) => m.key === args.key).brightness = args.pct;
        if (cmd === "set_confirm") state.confirm = { on: args.on, secs: args.secs };
        if (cmd === "set_identify") state.identify = { lit: args.lit, dim: args.dim };
        if (cmd === "set_positions") for (const [k, x, y] of args.list) Object.assign(state.monitors.find((m) => m.key === k), { x, y });
        if (cmd === "profile_hotkey" && args.hotkey === "Ctrl+KeyW") throw L("Cette combinaison est déjà prise par une autre application", "This combination is already taken by another app");
        if (cmd === "profile_apply" && args.id === "2") return L("Un écran du profil n'est pas branché", "One display from the profile is not connected");
        if (cmd === "profile_auto") state.profiles.find((p) => p.id === args.id).auto = args.on;
        if (cmd === "profile_triggers") state.profiles.find((p) => p.id === args.id).triggers = { ...args.triggers, at: args.triggers.at ?? null };
        if (cmd === "set_schedule") {
          const steps = [...args.schedule.steps].sort((a, b) => a[0] - b[0]);
          return structuredClone((state.schedule = { ...args.schedule, steps }));
        }
        if (cmd === "set_updates") state.updates = args.on;
        if (cmd === "check_update") return (state.update = "2.3.0");
        if (cmd === "download_update") state.update_step = "ready";
        if (cmd === "install_update") state.update_step = "install";
        if (cmd === "set_auto_update") state.auto_update = { ...args.auto };
        if (cmd === "open_releases") console.log("ouverture de la page des versions");
        if (cmd === "input_hotkey") state.screens[args.key].input_hotkey = args.hotkey;
        if (cmd === "profile_include") state.profiles.find((p) => p.id === args.id).extras.find((x) => x.id === args.setting).on = args.on;
        if (cmd === "set_tray_icon") state.tray_icon = args.name;
        if (cmd === "action_hotkey") state[{ "@off": "off_hotkey", "@up": "up_hotkey", "@down": "down_hotkey" }[args.action]] = args.hotkey;
        if (cmd === "ddc_values") return (state.screens[args.key]?.features ?? []).map((f) => ({ code: f.code, value: args.key === "c" ? null : ddc[f.code] ?? null }));
        if (cmd === "ddc_set") ddc[args.code] = args.value;
        if (cmd === "ddc_power") return (ddc[0xd6] = args.on ? 1 : 5);
        if (cmd === "ddc_caps") {
          if (args.key === "d" || args.key === "b") throw L("Cet écran ne répond pas en DDC/CI", "This display doesn't respond over DDC/CI");
          const sc = (state.screens[args.key] ??= screen({}));
          if (!Object.keys(sc.caps).length) sc.caps = aorus;
          return structuredClone(sc.caps);
        }
        if (cmd === "set_screen") return structuredClone((state.screens[args.key] = { ...args.screen, caps: state.screens[args.key]?.caps ?? {} }));
        if (cmd === "set_panel") state.panel = args.panel;
        if (cmd === "flyout_fit") document.body.style.height = `${args.h}px`;
        return null;
      },
    },
    // window.__emit("activity", {...}) simule un événement du backend.
    event: { listen: async (name, cb) => ((window.__events ??= {})[name] = cb, () => {}) },
    window: { getCurrentWindow: () => ({ close: () => console.log("fermeture de la fenêtre") }) },
  };
})();
