"use strict";
/**
 * Numéros des écrans qui montrent cette image : un seul, ou ceux d'écrans dupliqués.
 * `at` désigne celui qui reste lumineux pendant que les autres s'assombrissent.
 */
function show(id) {
  const nums = id.nums.map((n, i) => {
    const s = document.createElement("span");
    s.textContent = n;
    if (id.at != null && i !== id.at) s.className = "off";
    return s;
  });
  document.getElementById("n").replaceChildren(...nums);
  document.getElementById("name").textContent = id.at == null ? id.names.join(" + ") : id.names[id.at];
  document.getElementById("res").textContent = id.res;
}
if (window.ID) show(window.ID);
