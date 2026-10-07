/* ---------- DOM helper: never uses innerHTML, all data goes through text nodes ---------- */
export function h(tag, props, ...kids) {
  const e = document.createElement(tag);
  if (props) {
    for (const k of Object.keys(props)) {
      const v = props[k];
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") e.className = v;
      else if (k === "text") e.textContent = v;
      else if (k === "style") e.style.cssText = v;
      else if (typeof v === "function") e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : String(v));
    }
  }
  for (const c of kids) {
    if (c === null || c === undefined || c === false) continue;
    e.append(typeof c === "object" ? c : document.createTextNode(String(c)));
  }
  return e;
}

export const fmtTime = (s) => {
  if (s === null || s === undefined || s === "") return null;
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return String(s);
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
};
export const fmtClock = (ms) =>
  new Date(ms).toLocaleTimeString(undefined, { hour12: false });
export const shortId = (id) => String(id).slice(0, 8);
export const squash = (s) =>
  String(s === null || s === undefined ? "" : s)
    .replace(/\s+/g, " ")
    .trim();
export const cut = (s, n) => {
  s = squash(s);
  return s.length <= n ? s : s.slice(0, n - 1) + "…";
};
