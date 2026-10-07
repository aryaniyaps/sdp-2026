// Join the two API payloads without changing either source.
export function buildGraphData(proj, mem) {
  const wn = new Map();
  const wl = new Map();
  const inf = { unknownNodes: 0, badRelationships: 0, danglingSupports: 0 };
  if (proj) {
    for (const n of proj.nodes) {
      const cls =
        n.kind === "assertion" ? "fact" : n.kind === "entity" ? "entity" : null;
      if (!cls || typeof n.id !== "string") {
        inf.unknownNodes++;
        continue;
      }
      wn.set((cls === "fact" ? "f:" : "e:") + n.id, { cls, id: n.id, d: n });
    }
    for (const r of proj.relationships) {
      let type, fk, tk;
      if (r.type === "MENTIONS") {
        type = "mention";
        fk = "f:" + r.from;
        tk = "e:" + r.to;
      } else if (r.type === "SUPPORTS") {
        type = "rel";
        fk = "f:" + r.from;
        tk = "f:" + r.to;
      } else {
        inf.badRelationships++;
        continue;
      }
      if (!wn.has(fk) || !wn.has(tk)) {
        inf.badRelationships++;
        continue;
      }
      wl.set(type + "|" + fk + "|" + tk + "|" + (r.relation || ""), {
        type,
        from: fk,
        to: tk,
        relation: r.relation || null,
        explanation: r.explanation || null,
      });
    }
  }
  if (mem) {
    const dangling = new Set(); // one entry per memory and fact, however many quotes link them
    for (const m of mem.memories) {
      if (typeof m.id !== "string") {
        inf.unknownNodes++;
        continue;
      }
      const mk = "m:" + m.id;
      wn.set(mk, { cls: "memory", id: m.id, d: m });
      for (const s of m.supports || []) {
        if (!proj) continue; // the projection is unavailable, the error banner already says so
        const fk = "f:" + s.assertion_id;
        if (wn.has(fk))
          wl.set("ev|" + mk + "|" + fk + "|", {
            type: "ev",
            from: mk,
            to: fk,
            relation: null,
            explanation: null,
          });
        else dangling.add(mk + "|" + fk);
      }
    }
    inf.danglingSupports = dangling.size;
  }
  return { wn, wl, inf };
}
