import { h, fmtClock, shortId, squash, cut } from "./helpers.js";
import { buildGraphData } from "./data.js";
import { createDetails } from "./details.js";
import { createLayout } from "./layout.js";

export function mountGraph(container) {
  let disposed = false;
  const cleanups = [];
  const lifetime = new AbortController();
  function listen(target, event, listener, options) {
    target.addEventListener(event, listener, options);
    cleanups.push(() => target.removeEventListener(event, listener, options));
  }

  /* ---------- page parameters ---------- */
  const params = new URLSearchParams(location.search);
  const NS = params.get("namespace");
  const hasNs = NS !== null && NS.trim() !== "";
  const pageNotes = [];

  function intParam(name, lo, hi) {
    const raw = params.get(name);
    if (raw === null || raw === "") return null;
    const n = Number(raw);
    if (Number.isInteger(n) && n >= lo && n <= hi) return n;
    pageNotes.push(
      'Ignoring invalid "' +
        name +
        '" value "' +
        raw +
        '" (expected an integer from ' +
        lo +
        " to " +
        hi +
        ").",
    );
    return null;
  }
  const POLL_MIN_MS = 500;
  const pollParam = intParam("poll", 0, 3600000);
  if (pollParam !== null && pollParam > 0 && pollParam < POLL_MIN_MS)
    pageNotes.push(
      "poll is raised from " +
        pollParam +
        " ms to the minimum of " +
        POLL_MIN_MS +
        " ms.",
    );
  const POLL_MS =
    pollParam === null
      ? 2000
      : pollParam === 0
        ? 0
        : Math.max(POLL_MIN_MS, pollParam);
  const LIMIT_PROJECTION = intParam("limit", 1, 2000);
  const LIMIT_MEMORIES = intParam("memory_limit", 1, 1000);
  const REQUEST_TIMEOUT_MS = 15000;

  const $ = (id) => container.querySelector("#" + id);
  const clamp = (v, lo, hi) => (v < lo ? lo : v > hi ? hi : v);
  const cvs = $("cv");
  const ctx = cvs.getContext("2d", { alpha: false });
  const rootStyle = getComputedStyle(document.body);
  const cssVar = (n) => rootStyle.getPropertyValue(n).trim();
  const FONT_FAMILY = getComputedStyle(document.body).fontFamily;
  const LABEL_FONT = "13px " + FONT_FAMILY;
  const REL_FONT = "13px " + FONT_FAMILY;
  const COL = {
    memory: cssVar("--memory"),
    fact: cssVar("--fact"),
    entity: cssVar("--entity"),
    bg: cssVar("--bg"),
    amber: cssVar("--amber"),
  };
  const CLASSES = ["memory", "fact", "entity"];
  const TAU = Math.PI * 2;

  /* ---------- state ---------- */
  const state = {
    projection: null, // last good /projection payload
    memories: null, // last good /memories payload
    errors: { projection: null, memories: null },
    lastText: { projection: null, memories: null },
    lastFullRefresh: 0,
    okAt: { projection: 0, memories: 0 },
    cycleOk: null,
    loaded: false,
  };
  const layerSeen = { projection: false, memories: false };
  const classVisible = { memory: true, fact: true, entity: true };
  let info = { unknownNodes: 0, badRelationships: 0, danglingSupports: 0 };

  const nodes = new Map(); // key -> node
  const links = new Map(); // key -> link
  let nodeList = []; // all nodes, including fading ones
  let linkList = [];
  let simNodes = []; // nodes taking part in the layout
  let simLinks = [];
  let relLinks = []; // live fact to fact links
  let drawOrder = []; // nodes by radius, big first so small nodes stay on top
  let labelOrder = []; // nodes by degree, important first
  let evidence = new Map(); // fact id -> Map(memory key -> [quote])
  let selected = null; // node key
  let hover = null; // node
  let hoverLink = null;
  let searchTerm = "";
  let focusDirty = true;
  let focusActive = false;
  let drag = null; // {type: 'pan' | 'node', node, sx, sy, vx0, vy0, moved}
  let viewChanging = false;

  /* ---------- view transform ---------- */
  const view = { x: 0, y: 0, k: 1 };
  const viewT = { x: 0, y: 0, k: 1 };
  const K_MIN = 0.04,
    K_MAX = 8;
  let W = innerWidth,
    H = innerHeight,
    dpr = 1;
  let autoFit = true;
  let kp = 1; // node radius zoom factor
  const insets = { top: 60, bottom: 40, left: 14, right: 14 };

  /* ---------- simulation constants ---------- */
  const sim = {
    alpha: 1,
    alphaTarget: 0,
    active: false,
    acc: 0,
    nanResets: 0,
    spread: 1,
  };
  const { tick } = createLayout(sim);
  const ALPHA_MIN = 0.001;
  const STEP_MS = 1000 / 60;
  const LINK_DIST = { mention: 58, ev: 52, rel: 84 };

  /* ---------- link colours, bucketed by alpha so each bucket is one path ---------- */
  const LINK_RGB = {
    mention: [170, 176, 190],
    ev: [109, 211, 160],
    rel: [167, 139, 250],
    hl: [208, 196, 255],
  };
  const LINK_ALPHA = { mention: 0.4, ev: 0.42, rel: 0.62 };
  const LQ = 16;
  const BUCKET_TYPE = ["mention", "ev", "rel", "hl"];
  const LINK_BUCKET = { mention: 0, ev: 1, rel: 2 };
  const LINK_STYLE = {};
  for (const t of BUCKET_TYPE) {
    LINK_STYLE[t] = [];
    for (let i = 0; i <= LQ; i++)
      LINK_STYLE[t].push(
        "rgba(" + LINK_RGB[t].join(",") + "," + (i / LQ).toFixed(3) + ")",
      );
  }
  const linkBuckets = [];
  for (let i = 0; i < BUCKET_TYPE.length * (LQ + 1); i++) linkBuckets.push([]);

  /* ---------- nodes and links ---------- */
  function makeNode(key, cls, id, now) {
    return {
      key,
      cls,
      id,
      d: null,
      label: "",
      fullLabel: "",
      lw: 0,
      hay: "",
      x: 0,
      y: 0,
      vx: 0,
      vy: 0,
      fx: null,
      fy: null,
      placed: false,
      r: 5,
      q: 60,
      deg: 0,
      adj: [],
      ext: null,
      lpos: 1,
      ly: 0,
      born: now,
      pulse: false,
      v: 0,
      vt: 1,
      a: 1,
      ta: 1,
      la: 0,
      lt: 0,
      inFocus: false,
      useFull: false,
      observation: false,
      dimStatus: false,
      validToMs: null,
      match: true,
      dying: false,
    };
  }

  function setNodeData(n, d) {
    n.d = d;
    if (n.cls === "memory") {
      n.label = cut(d.text, 34);
      n.fullLabel = cut(d.text, 70);
      n.ext = d.extraction ? d.extraction.status : null;
      n.hay = (
        squash(d.text) +
        " " +
        (d.role || "") +
        " " +
        (d.processing_state || "") +
        " extraction " +
        (n.ext || "")
      ).toLowerCase();
    } else if (n.cls === "fact") {
      const name = d.name || d.statement || shortId(d.id);
      n.label = cut(name, 34);
      n.fullLabel = cut(d.statement || name, 70);
      n.hay = (
        squash(d.statement) +
        " " +
        squash(d.name) +
        " " +
        (d.status || "") +
        " " +
        (d.assertion_kind || "")
      ).toLowerCase();
      n.observation = d.assertion_kind === "observation";
      const t = d.valid_to ? Date.parse(d.valid_to) : NaN;
      n.validToMs = Number.isNaN(t) ? null : t;
    } else {
      const name = d.name || shortId(d.id);
      n.label = cut(name, 34);
      n.fullLabel = cut(name, 70);
      n.hay = (squash(name) + " " + (d.entity_type || "")).toLowerCase();
    }
    n.lw = 0;
    n.match = !searchTerm || n.hay.includes(searchTerm);
  }

  function isDim(n, wall) {
    if (n.cls !== "fact") return false;
    const st = n.d.status;
    if (st === "superseded" || st === "retracted" || st === "stale")
      return true;
    return n.validToMs !== null && n.validToMs <= wall;
  }

  function reconcile() {
    const now = performance.now();
    const wall = Date.now();
    const { wn, wl, inf } = buildGraphData(state.projection, state.memories);
    info = inf;
    const pulseOK = {
      memory: layerSeen.memories,
      fact: layerSeen.projection,
      entity: layerSeen.projection,
    };
    const added = [];
    let changed = false;

    for (const [key, w] of wn) {
      let n = nodes.get(key);
      if (!n) {
        n = makeNode(key, w.cls, w.id, now);
        n.pulse = pulseOK[w.cls];
        nodes.set(key, n);
        added.push(n);
      } else if (n.dying) {
        n.dying = false;
        changed = true;
      }
      setNodeData(n, w.d);
    }
    for (const [key, n] of nodes) {
      if (!wn.has(key) && !n.dying) {
        n.dying = true;
        changed = true;
      }
    }

    for (const [key, w] of wl) {
      let l = links.get(key);
      const s = nodes.get(w.from);
      const t = nodes.get(w.to);
      if (!l) {
        l = {
          key,
          type: w.type,
          s,
          t,
          relation: w.relation,
          explanation: w.explanation,
          v: 0,
          vt: 1,
          h: 0,
          ht: 0,
          dead: false,
          strength: 1,
          bias: 0.5,
          dist: 60,
        };
        links.set(key, l);
        changed = true;
      } else {
        if (l.dead) changed = true;
        l.s = s;
        l.t = t;
        l.relation = w.relation;
        l.explanation = w.explanation;
        l.dead = false;
      }
    }
    for (const [key, l] of links) {
      if (!wl.has(key) && !l.dead) {
        l.dead = true;
        changed = true;
      }
    }

    evidence = new Map();
    for (const w of wn.values()) {
      if (w.cls !== "memory") continue;
      const mk = "m:" + w.id;
      for (const s of w.d.supports || []) {
        let byMemory = evidence.get(s.assertion_id);
        if (!byMemory) evidence.set(s.assertion_id, (byMemory = new Map()));
        if (!byMemory.has(mk)) byMemory.set(mk, []);
        byMemory.get(mk).push(s.quote);
      }
    }

    rebuildTopology();
    placeNew(added);
    for (const n of nodeList) n.dimStatus = isDim(n, wall);
    layerSeen.projection = layerSeen.projection || !!state.projection;
    layerSeen.memories = layerSeen.memories || !!state.memories;

    if (added.length || changed) reheat(Math.min(1, 0.22 + added.length / 100));
    if (selected && (!nodes.has(selected) || nodes.get(selected).dying))
      selected = null;
    updateCounts();
    for (const n of nodeList)
      n.match = !searchTerm || n.hay.includes(searchTerm);
    updateMatchCount();
    focusDirty = true;
    refreshDetails();
    invalidate();
  }

  function applyVisibility() {
    for (const n of nodeList) n.vt = !n.dying && classVisible[n.cls] ? 1 : 0;
    for (const l of linkList)
      l.vt =
        !l.dead &&
        !l.s.dying &&
        !l.t.dying &&
        classVisible[l.s.cls] &&
        classVisible[l.t.cls]
          ? 1
          : 0;
    simNodes = nodeList.filter((n) => n.vt === 1);
    simLinks = linkList.filter((l) => l.vt === 1);
    /* small graphs get longer links so they do not collapse into a blob */
    sim.spread = 1 + 0.9 * clamp(1 - (simNodes.length - 6) / 60, 0, 1);
  }

  function rebuildTopology() {
    nodeList = Array.from(nodes.values());
    linkList = Array.from(links.values());
    for (const n of nodeList) n.adj = [];
    relLinks = [];
    for (const l of linkList) {
      if (l.dead || l.s.dying || l.t.dying) continue;
      l.s.adj.push(l);
      l.t.adj.push(l);
      if (l.type === "rel") relLinks.push(l);
    }
    for (const n of nodeList) {
      n.deg = n.adj.length;
      n.r = Math.min(
        16,
        n.cls === "entity"
          ? 4.6 + 2.1 * Math.sqrt(n.deg)
          : 3.5 + 1.7 * Math.sqrt(n.deg),
      );
      n.q = 48 + 13 * Math.sqrt(n.deg);
    }
    for (const l of linkList) {
      const ds = l.s.deg || 1,
        dt = l.t.deg || 1;
      l.strength = 1 / Math.min(ds, dt);
      l.bias = ds / (ds + dt);
      l.dist = LINK_DIST[l.type];
    }
    drawOrder = nodeList.slice().sort((a, b) => b.r - a.r);
    labelOrder = nodeList.slice().sort((a, b) => b.deg - a.deg);
    applyVisibility();
  }

  /* new nodes appear next to a linked neighbour, or near the centre when they have none */
  function placeNew(added) {
    if (!added.length) return;
    const existing = nodeList.some((n) => n.placed);
    const pending = new Set(added);
    const queue = [];
    let seeds = 0;
    const put = (n, x, y) => {
      n.x = x;
      n.y = y;
      n.vx = 0;
      n.vy = 0;
      n.placed = true;
      pending.delete(n);
      queue.push(n);
    };
    const nearAnchor = (n, a) => {
      const ang = Math.random() * TAU;
      const dist = 26 + Math.random() * 22;
      put(n, a.x + Math.cos(ang) * dist, a.y + Math.sin(ang) * dist);
    };
    const other = (l, n) => (l.s === n ? l.t : l.s);
    for (const n of added) {
      const a = n.adj.map((l) => other(l, n)).find((o) => o.placed && o !== n);
      if (a) nearAnchor(n, a);
    }
    const drain = () => {
      while (queue.length) {
        const a = queue.shift();
        for (const l of a.adj) {
          const o = other(l, a);
          if (pending.has(o)) nearAnchor(o, a);
        }
      }
    };
    drain();
    while (pending.size) {
      let best = null;
      for (const n of pending) if (!best || n.deg > best.deg) best = n;
      let x = 0,
        y = 0;
      if (existing || seeds > 0) {
        const rad = 42 * Math.sqrt(seeds + (existing ? 0 : 0.5));
        const ang = seeds * 2.399963;
        x = Math.cos(ang) * rad + (Math.random() - 0.5) * 16;
        y = Math.sin(ang) * rad + (Math.random() - 0.5) * 16;
      }
      seeds++;
      put(best, x, y);
      drain();
    }
  }

  function reheat(a) {
    if (sim.alpha < a) sim.alpha = a;
    sim.active = true;
  }

  function updateCounts() {
    const c = { memory: 0, fact: 0, entity: 0 };
    for (const n of nodeList) if (!n.dying) c[n.cls]++;
    for (const k of CLASSES) $("c-" + k).textContent = String(c[k]);
    cvs.setAttribute(
      "aria-label",
      "Memory graph with " +
        c.memory +
        " memories, " +
        c.fact +
        " facts and " +
        c.entity +
        " entities",
    );
  }

  /* ---------- picking ---------- */
  const pointer = { x: -1, y: -1, inside: false, moved: false };

  function screenRadius(n) {
    return Math.max(2.4, n.r * kp);
  }

  function pickNode(x, y) {
    kp = Math.pow(view.k, 0.6);
    for (let i = drawOrder.length - 1; i >= 0; i--) {
      const n = drawOrder[i];
      if (n.v < 0.3) continue;
      const dx = view.x + n.x * view.k - x,
        dy = view.y + n.y * view.k - y;
      const r = screenRadius(n) + 4;
      if (dx * dx + dy * dy <= r * r) return n;
    }
    return null;
  }

  function pickLink(x, y) {
    let best = null,
      bestD = 36;
    for (const l of relLinks) {
      if (l.v < 0.3 || l.s.v < 0.3 || l.t.v < 0.3) continue;
      const ax = view.x + l.s.x * view.k,
        ay = view.y + l.s.y * view.k;
      const bx = view.x + l.t.x * view.k,
        by = view.y + l.t.y * view.k;
      const vx = bx - ax,
        vy = by - ay;
      const len2 = vx * vx + vy * vy;
      const t = len2 ? clamp(((x - ax) * vx + (y - ay) * vy) / len2, 0, 1) : 0;
      const dx = ax + vx * t - x,
        dy = ay + vy * t - y;
      const d = dx * dx + dy * dy;
      if (d < bestD) {
        bestD = d;
        best = l;
      }
    }
    return best;
  }

  /* ---------- focus: hover highlight and search dimming ---------- */
  function updateFocus() {
    focusDirty = false;
    let set = null;
    if (hover) {
      set = new Set([hover]);
      for (const l of hover.adj) set.add(l.s === hover ? l.t : l.s);
    } else if (hoverLink) {
      set = new Set([hoverLink.s, hoverLink.t]);
    }
    focusActive = !!set;
    for (const n of nodeList) {
      let t = 1;
      if (set && !set.has(n)) t = 0.1;
      if (searchTerm && !n.match) t = Math.min(t, 0.14);
      if (n === hover) t = 1;
      n.ta = t;
      n.inFocus = set ? set.has(n) : false;
    }
    for (const l of linkList)
      l.ht =
        (hover && (l.s === hover || l.t === hover)) || l === hoverLink ? 1 : 0;
  }

  /* ---------- drawing ---------- */
  const DASH = [2.5, 2.5];
  const NODASH = [];
  const LABEL_TEXT_H = 15;
  const rectBuf = new Float32Array(4 * 600);
  let rectCount = 0;
  function easeOutBack(t) {
    const c1 = 1.70158,
      c3 = c1 + 1;
    return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
  }

  function nodePath(n, sx, sy, r) {
    if (n.observation) {
      const d = r * 1.28;
      ctx.moveTo(sx, sy - d);
      ctx.lineTo(sx + d, sy);
      ctx.lineTo(sx, sy + d);
      ctx.lineTo(sx - d, sy);
      ctx.closePath();
    } else {
      ctx.moveTo(sx + r, sy);
      ctx.arc(sx, sy, r, 0, TAU);
    }
  }

  /* zoom level at which a node label starts to fade in: hubs first, and later when the graph is dense */
  function labelThreshold(n, density) {
    /* never below 0.3, so labels fade out when zooming far out, whatever the graph size or the degree */
    return Math.max(
      0.3,
      (n.cls === "memory" ? 1.4 : n.cls === "entity" ? 0.9 : 1.0) +
        density -
        0.1 * Math.min(n.deg, 10),
    );
  }

  function visRadius(n) {
    return screenRadius(n) * (n.observation ? 1.28 : 1);
  }

  /* screen circles of the nodes, on a coarse grid, so a label can avoid covering another node */
  const CIRC_CELL = 32;
  let circBuf = new Float32Array(3 * 512),
    circNext = new Int32Array(512),
    circHead = new Int32Array(0);
  let circCols = 0,
    circRows = 0,
    circMaxR = 0;

  function buildCircleGrid() {
    circCols = Math.ceil(W / CIRC_CELL) + 1;
    circRows = Math.ceil(H / CIRC_CELL) + 1;
    if (circCols * circRows > circHead.length)
      circHead = new Int32Array(circCols * circRows);
    circHead.fill(-1, 0, circCols * circRows);
    if (drawOrder.length > circNext.length) {
      circNext = new Int32Array(Math.ceil(drawOrder.length * 1.5));
      circBuf = new Float32Array(3 * circNext.length);
    }
    circMaxR = 0;
    let count = 0;
    for (let i = 0; i < drawOrder.length; i++) {
      const n = drawOrder[i];
      if (n.v < 0.3) continue;
      const sx = view.x + n.x * view.k,
        sy = view.y + n.y * view.k;
      if (sx < 0 || sx >= W || sy < 0 || sy >= H) continue;
      const r = visRadius(n) + 1.5;
      if (r > circMaxR) circMaxR = r;
      circBuf[count * 3] = sx;
      circBuf[count * 3 + 1] = sy;
      circBuf[count * 3 + 2] = r;
      const cell =
        Math.floor(sy / CIRC_CELL) * circCols + Math.floor(sx / CIRC_CELL);
      circNext[count] = circHead[cell];
      circHead[cell] = count++;
    }
  }

  /* true when no node other than the one at (ox, oy) touches the rectangle */
  function circlesFree(x0, y0, x1, y1, ox, oy) {
    const c0 = Math.max(0, Math.floor((x0 - circMaxR) / CIRC_CELL)),
      c1 = Math.min(circCols - 1, Math.floor((x1 + circMaxR) / CIRC_CELL));
    const r0 = Math.max(0, Math.floor((y0 - circMaxR) / CIRC_CELL)),
      r1 = Math.min(circRows - 1, Math.floor((y1 + circMaxR) / CIRC_CELL));
    for (let r = r0; r <= r1; r++) {
      for (let c = c0; c <= c1; c++) {
        for (let i = circHead[r * circCols + c]; i >= 0; i = circNext[i]) {
          const cx = circBuf[i * 3],
            cy = circBuf[i * 3 + 1],
            cr = circBuf[i * 3 + 2];
          if (cx === ox && cy === oy) continue;
          const dx = cx - (cx < x0 ? x0 : cx > x1 ? x1 : cx),
            dy = cy - (cy < y0 ? y0 : cy > y1 ? y1 : cy);
          if (dx * dx + dy * dy < cr * cr) return false;
        }
      }
    }
    return true;
  }

  function rectFree(x0, y0, x1, y1) {
    for (let i = 0; i < rectCount; i++) {
      const o = i * 4;
      if (
        x0 < rectBuf[o + 2] &&
        x1 > rectBuf[o] &&
        y0 < rectBuf[o + 3] &&
        y1 > rectBuf[o + 1]
      )
        return false;
    }
    return true;
  }

  function linkAlpha(l) {
    const mv = Math.min(l.s.v, l.t.v);
    const normal = LINK_ALPHA[l.type] * Math.min(l.s.a, l.t.a) * mv * l.v;
    return normal * (1 - l.h) + 0.9 * l.h * l.v * mv;
  }

  function draw(now, dt) {
    const k = view.k;
    kp = Math.pow(k, 0.6);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalAlpha = 1;
    ctx.fillStyle = COL.bg;
    ctx.fillRect(0, 0, W, H);

    for (let i = 0; i < linkBuckets.length; i++) linkBuckets[i].length = 0;
    for (const l of linkList) {
      if (l.v < 0.01) continue;
      const x1 = view.x + l.s.x * k,
        y1 = view.y + l.s.y * k,
        x2 = view.x + l.t.x * k,
        y2 = view.y + l.t.y * k;
      if (
        (x1 < -20 && x2 < -20) ||
        (x1 > W + 20 && x2 > W + 20) ||
        (y1 < -20 && y2 < -20) ||
        (y1 > H + 20 && y2 > H + 20)
      )
        continue;
      const al = linkAlpha(l);
      if (al < 0.01) continue;
      const cat = l.h > 0.5 ? 3 : LINK_BUCKET[l.type];
      linkBuckets[cat * (LQ + 1) + Math.min(LQ, Math.round(al * LQ))].push(l);
    }
    for (let b = 0; b < linkBuckets.length; b++) {
      const list = linkBuckets[b];
      if (!list.length) continue;
      const cat = (b / (LQ + 1)) | 0;
      ctx.strokeStyle = LINK_STYLE[BUCKET_TYPE[cat]][b % (LQ + 1)];
      ctx.lineWidth = cat === 3 ? 1.5 : 1;
      ctx.beginPath();
      for (let i = 0; i < list.length; i++) {
        const l = list[i];
        ctx.moveTo(view.x + l.s.x * k, view.y + l.s.y * k);
        ctx.lineTo(view.x + l.t.x * k, view.y + l.t.y * k);
      }
      ctx.stroke();
    }

    /* arrowheads on fact to fact links, direction matters for relations such as supersedes */
    for (const l of relLinks) {
      if (l.v < 0.05) continue;
      const al = linkAlpha(l);
      if (al < 0.04) continue;
      const x1 = view.x + l.s.x * k,
        y1 = view.y + l.s.y * k,
        x2 = view.x + l.t.x * k,
        y2 = view.y + l.t.y * k;
      const dx = x2 - x1,
        dy = y2 - y1;
      const len = Math.hypot(dx, dy);
      if (len < 18) continue;
      const ux = dx / len,
        uy = dy / len;
      const tipD = screenRadius(l.t) * (l.t.observation ? 1.28 : 1) + 2;
      const tx = x2 - ux * tipD,
        ty = y2 - uy * tipD;
      ctx.globalAlpha = Math.min(1, al * 1.4);
      ctx.fillStyle = l.h > 0.5 ? LINK_STYLE.hl[LQ] : LINK_STYLE.rel[LQ];
      ctx.beginPath();
      ctx.moveTo(tx, ty);
      ctx.lineTo(tx - ux * 7 - uy * 3.4, ty - uy * 7 + ux * 3.4);
      ctx.lineTo(tx - ux * 7 + uy * 3.4, ty - uy * 7 - ux * 3.4);
      ctx.closePath();
      ctx.fill();
    }
    ctx.globalAlpha = 1;

    for (let i = 0; i < drawOrder.length; i++) {
      const n = drawOrder[i];
      if (n.v < 0.01) continue;
      const sx = view.x + n.x * k,
        sy = view.y + n.y * k;
      if (sx < -40 || sx > W + 40 || sy < -40 || sy > H + 40) continue;
      const age = now - n.born;
      const r =
        screenRadius(n) *
        (age >= 420 ? 1 : age <= 0 ? 0 : easeOutBack(age / 420));
      if (r <= 0) continue;
      const al = n.v * n.a * (n.dimStatus ? 0.42 : 1);
      ctx.globalAlpha = al;
      ctx.fillStyle = COL[n.cls];
      ctx.beginPath();
      nodePath(n, sx, sy, r);
      ctx.fill();
      if (n.dimStatus) {
        ctx.setLineDash(DASH);
        ctx.lineWidth = 1;
        ctx.strokeStyle = COL[n.cls];
        ctx.globalAlpha = Math.min(1, al * 1.9);
        ctx.beginPath();
        ctx.arc(sx, sy, r + 3.6, 0, TAU);
        ctx.stroke();
        ctx.setLineDash(NODASH);
      }
      if (n.ext === "failed" || n.ext === "blocked") {
        ctx.setLineDash(n.ext === "blocked" ? DASH : NODASH);
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = COL.amber;
        ctx.globalAlpha = Math.min(1, al * 1.2);
        ctx.beginPath();
        ctx.arc(sx, sy, r + 3.6, 0, TAU);
        ctx.stroke();
        ctx.setLineDash(NODASH);
      }
      if (n.fx !== null) {
        ctx.globalAlpha = al * 0.9;
        ctx.fillStyle = COL.bg;
        ctx.beginPath();
        ctx.arc(sx, sy, Math.max(0.9, r * 0.22), 0, TAU);
        ctx.fill();
      }
    }

    ctx.lineWidth = 2;
    for (let i = 0; i < nodeList.length; i++) {
      const n = nodeList[i];
      if (!n.pulse) continue;
      const age = now - n.born;
      if (age < 0 || age > 1100) continue;
      const p = age / 1100;
      ctx.globalAlpha = (1 - p) * (1 - p) * 0.85 * n.v;
      ctx.strokeStyle = COL[n.cls];
      ctx.beginPath();
      ctx.arc(
        view.x + n.x * k,
        view.y + n.y * k,
        screenRadius(n) + 4 + p * 26,
        0,
        TAU,
      );
      ctx.stroke();
    }

    const ring = (n, extra, color, w, a) => {
      ctx.globalAlpha = a * n.v;
      ctx.strokeStyle = color;
      ctx.lineWidth = w;
      ctx.beginPath();
      ctx.arc(
        view.x + n.x * k,
        view.y + n.y * k,
        screenRadius(n) + extra,
        0,
        TAU,
      );
      ctx.stroke();
    };
    if (hover && hover.v > 0.1) ring(hover, 3, "#ffffff", 1, 0.55);
    const sn = selected ? nodes.get(selected) : null;
    if (sn && sn.v > 0.1) ring(sn, 4.5, "#ffffff", 1.5, 0.9);
    ctx.globalAlpha = 1;

    drawLabels(dt);
    drawRelationLabels();
  }

  function drawLabels(dt) {
    ctx.font = LABEL_FONT;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    rectCount = 0;
    buildCircleGrid();
    const k = view.k;
    let visibleCount = 0;
    for (const n of nodeList) {
      n.lt = 0;
      if (n.vt === 1) visibleCount++;
    }
    const density =
      visibleCount > 40 ? 0.4 * clamp((visibleCount - 40) / 250, 0, 1) : -10;

    /* a label goes under its node, or above it when that spot is taken by another label or a node */
    const place = (n, force, fade) => {
      if (n.v < 0.2) return;
      const sx = view.x + n.x * k,
        sy = view.y + n.y * k;
      if (sx < -80 || sx > W + 80 || sy < -30 || sy > H + 30) return;
      if (!n.lw) n.lw = ctx.measureText(n.label).width;
      const w =
        force && n.fullLabel !== n.label
          ? ctx.measureText(n.fullLabel).width
          : n.lw;
      const vr = visRadius(n);
      const x0 = sx - w / 2 - 3,
        x1 = sx + w / 2 + 3;
      const first = n.lpos === -1 ? -1 : 1;
      for (let t = 0; t < 2; t++) {
        const side = t === 0 ? first : -first;
        const y = side === 1 ? sy + vr + 4 : sy - vr - 4 - LABEL_TEXT_H;
        const y0 = y - 1,
          y1 = y + 17;
        if (
          !force &&
          !(rectFree(x0, y0, x1, y1) && circlesFree(x0, y0, x1, y1, sx, sy))
        )
          continue;
        if (rectCount < 600) {
          const o = rectCount++ * 4;
          rectBuf[o] = x0;
          rectBuf[o + 1] = y0;
          rectBuf[o + 2] = x1;
          rectBuf[o + 3] = y1;
        }
        n.lt = fade;
        n.useFull = force;
        n.lpos = side;
        n.ly = y - sy;
        return;
      }
    };

    /* priority labels first: hovered node, selected node, then the rest by importance */
    if (hover) place(hover, true, 1);
    const sn = selected ? nodes.get(selected) : null;
    if (sn && sn !== hover) place(sn, true, 1);
    if (focusActive) {
      for (const n of nodeList) if (n.inFocus && n.lt === 0) place(n, false, 1);
    } else if (searchTerm) {
      let c = 0;
      for (const n of labelOrder)
        if (n.match && n.lt === 0 && c < 150 && classVisible[n.cls]) {
          place(n, false, 1);
          c++;
        }
    } else {
      for (const n of labelOrder) {
        if (n.lt !== 0) continue;
        const f = clamp((k - labelThreshold(n, density)) / 0.22, 0, 1);
        if (f > 0.02) place(n, false, f);
      }
    }

    const ease = 1 - Math.exp(-dt / 90);
    ctx.lineJoin = "round";
    ctx.lineWidth = 3.5;
    for (const n of nodeList) {
      if (n.la !== n.lt) {
        n.la += (n.lt - n.la) * ease;
        if (Math.abs(n.la - n.lt) < 0.01) n.la = n.lt;
      }
      if (n.la < 0.02) continue;
      const sx = view.x + n.x * k,
        sy = view.y + n.y * k;
      const text = n.useFull && n.lt === 1 ? n.fullLabel : n.label;
      const y = sy + n.ly;
      const a =
        n.la *
        n.v *
        Math.max(0.35, n.a) *
        (n.dimStatus && n !== hover ? 0.65 : 1);
      ctx.globalAlpha = a * 0.85;
      ctx.strokeStyle = COL.bg;
      ctx.strokeText(text, sx, y);
      ctx.globalAlpha = a;
      ctx.fillStyle = n === hover ? "#ffffff" : "#c9c9c9";
      ctx.fillText(text, sx, y);
    }
    ctx.globalAlpha = 1;
  }

  function drawRelationLabels() {
    ctx.font = REL_FONT;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    const placed = [];
    const order = relLinks
      .filter((l) => l.h >= 0.5 && l.relation)
      .sort((a, b) => (b === hoverLink) - (a === hoverLink));
    for (const l of order) {
      const w = ctx.measureText(l.relation).width + 14;
      let spot = null;
      for (const t of [0.5, 0.35, 0.65, 0.25, 0.75]) {
        const mx = view.x + (l.s.x + (l.t.x - l.s.x) * t) * view.k,
          my = view.y + (l.s.y + (l.t.y - l.s.y) * t) * view.k;
        if (
          !placed.some(
            (p) =>
              Math.abs(p[0] - mx) < (p[2] + w) / 2 && Math.abs(p[1] - my) < 24,
          )
        ) {
          spot = [mx, my, w];
          break;
        }
      }
      if (!spot) continue;
      placed.push(spot);
      ctx.globalAlpha = l.h;
      ctx.fillStyle = "#2b2740";
      ctx.strokeStyle = "rgba(167,139,250,0.75)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      if (ctx.roundRect)
        ctx.roundRect(spot[0] - w / 2, spot[1] - 11, w, 22, 11);
      else ctx.rect(spot[0] - w / 2, spot[1] - 11, w, 22);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = "#e4dcff";
      ctx.fillText(l.relation, spot[0], spot[1] + 0.5);
    }
    ctx.globalAlpha = 1;
  }

  /* ---------- frame loop: runs only while something moves ---------- */
  let raf = 0,
    lastT = 0;

  function invalidate() {
    if (!disposed && !raf) raf = requestAnimationFrame(frame);
  }

  function frame(t) {
    raf = 0;
    if (
      window.devicePixelRatio !== dpr ||
      innerWidth !== W ||
      innerHeight !== H
    )
      resize();
    const dt = clamp(lastT ? t - lastT : 16.7, 1, 64);
    lastT = t;
    const now = performance.now();
    const wall = Date.now();
    let busy = false;

    /* drop nodes and links that have finished fading out */
    let removed = false;
    for (const [key, n] of nodes) {
      if (n.dying && n.v < 0.02) {
        nodes.delete(key);
        removed = true;
      }
    }
    for (const [key, l] of links) {
      if (
        (l.dead || !nodes.has(l.s.key) || !nodes.has(l.t.key)) &&
        l.v < 0.02
      ) {
        links.delete(key);
        removed = true;
      }
    }
    if (removed) {
      rebuildTopology();
      updateCounts();
      if (hover && !nodes.has(hover.key)) hover = null;
      focusDirty = true;
    }
    for (const n of nodeList) n.dimStatus = isDim(n, wall);

    if (sim.active) {
      sim.acc += dt;
      let steps = 0;
      while (sim.acc >= STEP_MS && steps < 3 && sim.active) {
        tick(simNodes, simLinks);
        sim.acc -= STEP_MS;
        steps++;
      }
      if (sim.acc > STEP_MS * 3) sim.acc = 0;
      busy = sim.active;
    }

    if (
      !drag &&
      pointer.inside &&
      (pointer.moved || sim.active || viewChanging)
    ) {
      pointer.moved = false;
      const hit = pickNode(pointer.x, pointer.y);
      const hl = hit ? null : pickLink(pointer.x, pointer.y);
      if (hit !== hover || hl !== hoverLink) {
        hover = hit;
        hoverLink = hl;
        focusDirty = true;
        cvs.style.cursor = hit ? "pointer" : "default";
        showLinkTip(hl);
      }
    }
    if (focusDirty) updateFocus();

    const eNode = 1 - Math.exp(-dt / 70);
    const eVis = 1 - Math.exp(-dt / 110);
    const eLink = 1 - Math.exp(-dt / 80);
    for (const n of nodeList) {
      if (n.a !== n.ta) {
        n.a += (n.ta - n.a) * eNode;
        if (Math.abs(n.a - n.ta) < 0.01) n.a = n.ta;
        busy = true;
      }
      if (n.v !== n.vt) {
        n.v += (n.vt - n.v) * eVis;
        if (Math.abs(n.v - n.vt) < 0.01) n.v = n.vt;
        busy = true;
      }
      if (n.la !== n.lt || now - n.born < (n.pulse ? 1150 : 450)) busy = true;
    }
    for (const l of linkList) {
      if (l.v !== l.vt) {
        l.v += (l.vt - l.v) * eVis;
        if (Math.abs(l.v - l.vt) < 0.01) l.v = l.vt;
        busy = true;
      }
      if (l.h !== l.ht) {
        l.h += (l.ht - l.h) * eLink;
        if (Math.abs(l.h - l.ht) < 0.01) l.h = l.ht;
        busy = true;
      }
    }

    if (autoFit && sim.active) fitView(false);
    viewChanging = false;
    const dk = viewT.k - view.k,
      dx = viewT.x - view.x,
      dy = viewT.y - view.y;
    if (Math.abs(dk) > 0.0005 || Math.abs(dx) > 0.15 || Math.abs(dy) > 0.15) {
      const e = 1 - Math.exp(-dt / 85);
      view.k += dk * e;
      view.x += dx * e;
      view.y += dy * e;
      viewChanging = true;
      busy = true;
    } else {
      view.k = viewT.k;
      view.x = viewT.x;
      view.y = viewT.y;
    }

    draw(now, dt);

    if (busy || drag || viewChanging) raf = requestAnimationFrame(frame);
    else lastT = 0;
  }

  /* ---------- canvas size and fitting ---------- */
  function resize() {
    dpr = window.devicePixelRatio || 1;
    W = innerWidth;
    H = innerHeight;
    cvs.width = Math.max(1, Math.round(W * dpr));
    cvs.height = Math.max(1, Math.round(H * dpr));
    measureInsets();
    if (autoFit) fitView(false);
    invalidate();
  }

  function measureInsets() {
    const top = $("top").getBoundingClientRect();
    const bottom = $("bottom").getBoundingClientRect();
    insets.top = top.bottom + 14;
    insets.bottom = Math.max(0, H - bottom.top) + 14;
    insets.left = 14;
    insets.right = 14;
    const d = $("details");
    if (d.classList.contains("show"))
      insets.right = Math.max(14, W - d.getBoundingClientRect().left + 14);
  }

  function fitView(snap) {
    let minX = Infinity,
      minY = Infinity,
      maxX = -Infinity,
      maxY = -Infinity;
    for (const n of simNodes) {
      if (n.x - n.r < minX) minX = n.x - n.r;
      if (n.x + n.r > maxX) maxX = n.x + n.r;
      if (n.y - n.r < minY) minY = n.y - n.r;
      if (n.y + n.r + 18 > maxY) maxY = n.y + n.r + 18;
    }
    if (!simNodes.length) return;
    const aw = Math.max(80, W - insets.left - insets.right);
    const ah = Math.max(80, H - insets.top - insets.bottom);
    const bw = Math.max(40, maxX - minX),
      bh = Math.max(40, maxY - minY);
    const cap = 1.5 + 1.0 * clamp(1 - (simNodes.length - 6) / 150, 0, 1);
    const k = clamp(Math.min(aw / bw, ah / bh) * 0.96, K_MIN, cap);
    viewT.k = k;
    viewT.x = insets.left + aw / 2 - ((minX + maxX) / 2) * k;
    viewT.y = insets.top + ah / 2 - ((minY + maxY) / 2) * k;
    if (snap) {
      view.k = viewT.k;
      view.x = viewT.x;
      view.y = viewT.y;
    }
  }

  function centerOn(n) {
    const k = Math.max(viewT.k, 0.8);
    viewT.k = k;
    viewT.x = insets.left + (W - insets.left - insets.right) / 2 - n.x * k;
    viewT.y = insets.top + (H - insets.top - insets.bottom) / 2 - n.y * k;
    autoFit = false;
    invalidate();
  }

  /* ---------- pointer interaction ---------- */
  /* A double click is found here, not with the dblclick event: the first click opens the details
     panel and may move the view, so the second click can land on neither the node nor the canvas.
     The second press counts when it is close to the first click in time and place, whatever is
     under it now. */
  const DBL_MS = 450,
    DBL_PX = 10;
  let lastClick = null; // {key, x, y, t} of the last click on a node
  let settleTimer = 0;

  function endSettle() {
    clearTimeout(settleTimer);
    details.classList.remove("settle");
  }

  function releaseNode(n) {
    if (n.fx === null) return;
    n.fx = null;
    n.fy = null;
    reheat(0.3);
    invalidate();
  }

  listen(cvs, "pointerdown", (e) => {
    if (e.button !== 0) return;
    cvs.setPointerCapture(e.pointerId);
    pointer.x = e.clientX;
    pointer.y = e.clientY;
    pointer.inside = true;
    autoFit = false;
    showLinkTip(null);
    if (
      lastClick &&
      performance.now() - lastClick.t <= DBL_MS &&
      Math.hypot(e.clientX - lastClick.x, e.clientY - lastClick.y) <= DBL_PX
    ) {
      const n = nodes.get(lastClick.key);
      lastClick = null;
      if (n && !n.dying) releaseNode(n);
      drag = {
        type: "double",
        node: null,
        sx: e.clientX,
        sy: e.clientY,
        vx0: view.x,
        vy0: view.y,
        moved: false,
      };
      invalidate();
      return;
    }
    const hit = pickNode(e.clientX, e.clientY);
    drag = {
      type: hit ? "node" : "pan",
      node: hit,
      sx: e.clientX,
      sy: e.clientY,
      vx0: view.x,
      vy0: view.y,
      moved: false,
    };
    invalidate();
  });

  listen(cvs, "pointermove", (e) => {
    pointer.x = e.clientX;
    pointer.y = e.clientY;
    pointer.inside = true;
    pointer.moved = true;
    /* a pointer that has left the click spot is not heading for a second click, so the panel takes events again */
    if (
      lastClick &&
      Math.hypot(e.clientX - lastClick.x, e.clientY - lastClick.y) > DBL_PX
    )
      endSettle();
    if (drag && drag.type === "double") return;
    if (drag) {
      const mx = e.clientX - drag.sx,
        my = e.clientY - drag.sy;
      if (!drag.moved && mx * mx + my * my > 9) {
        drag.moved = true;
        cvs.style.cursor = "grabbing";
        if (drag.type === "node") {
          sim.alphaTarget = 0.3;
          sim.active = true;
        }
      }
      if (drag.moved) {
        if (drag.type === "pan") {
          view.x = viewT.x = drag.vx0 + mx;
          view.y = viewT.y = drag.vy0 + my;
        } else {
          const n = drag.node;
          n.fx = (e.clientX - view.x) / view.k;
          n.fy = (e.clientY - view.y) / view.k;
          n.x = n.fx;
          n.y = n.fy;
        }
      }
    } else {
      moveTip(e.clientX, e.clientY);
    }
    invalidate();
  });

  function endDrag(cancelled) {
    if (!drag) return;
    const d = drag;
    drag = null;
    sim.alphaTarget = 0;
    cvs.style.cursor = hover ? "pointer" : "default";
    if (d.type !== "double") {
      lastClick = null;
      if (!d.moved && !cancelled) {
        select(d.node ? d.node.key : null, false);
        if (d.node) {
          lastClick = {
            key: d.node.key,
            x: d.sx,
            y: d.sy,
            t: performance.now(),
          };
          details.classList.add("settle");
          clearTimeout(settleTimer);
          settleTimer = setTimeout(endSettle, DBL_MS);
        }
      }
    }
    pointer.moved = true;
    invalidate();
  }
  listen(cvs, "pointerup", () => endDrag(false));
  listen(cvs, "pointercancel", () => endDrag(true));
  listen(cvs, "pointerleave", () => {
    pointer.inside = false;
    if (!drag && (hover || hoverLink)) {
      hover = null;
      hoverLink = null;
      focusDirty = true;
      cvs.style.cursor = "default";
      showLinkTip(null);
      invalidate();
    }
  });
  listen(
    cvs,
    "wheel",
    (e) => {
      e.preventDefault();
      autoFit = false;
      const delta =
        e.deltaMode === 1
          ? e.deltaY * 16
          : e.deltaMode === 2
            ? e.deltaY * 400
            : e.deltaY;
      const k2 = clamp(
        viewT.k * Math.exp(-delta * (e.ctrlKey ? 0.01 : 0.0016)),
        K_MIN,
        K_MAX,
      );
      const wx = (e.clientX - viewT.x) / viewT.k,
        wy = (e.clientY - viewT.y) / viewT.k;
      viewT.k = k2;
      viewT.x = e.clientX - wx * k2;
      viewT.y = e.clientY - wy * k2;
      invalidate();
    },
    { passive: false },
  );

  /* ---------- tooltip for relation links ---------- */
  const tip = $("tip");
  function showLinkTip(l) {
    if (!l || !(l.relation || l.explanation)) {
      tip.style.display = "none";
      return;
    }
    tip.replaceChildren(
      h("b", { text: l.relation || "related" }),
      l.explanation ? ": " + l.explanation : "",
    );
    tip.style.display = "block";
    moveTip(pointer.x, pointer.y);
  }
  function moveTip(x, y) {
    if (tip.style.display === "none") return;
    tip.style.transform =
      "translate(" +
      Math.max(8, Math.min(x + 14, W - 340)) +
      "px," +
      (y + 16) +
      "px)";
  }

  /* ---------- selection and details panel ---------- */
  const details = $("details");
  const dbody = $("dbody");
  const dtitle = $("dtitle");
  let lastModelSig = null;
  let lastDetailKey = null;
  let panelCollapsed = false;

  function select(key, center) {
    selected = key;
    if (key) {
      panelCollapsed = false;
      if (center) {
        const n = nodes.get(key);
        if (n) centerOn(n);
      }
    }
    refreshDetails();
    if (key && !center) {
      const n = nodes.get(key);
      if (n) revealNode(n);
    }
    invalidate();
  }

  /* The panel overlays the right side. When it opens over the node that was just clicked, or over
     its label, pan the view left by the least amount that frees the node. */
  function revealNode(n) {
    const r = details.getBoundingClientRect();
    if (!r.width) return;
    const sx = viewT.x + n.x * viewT.k,
      sy = viewT.y + n.y * viewT.k;
    if (sy < r.top - 24 || sy > r.bottom + 24) return;
    ctx.font = LABEL_FONT;
    const half =
      Math.max(
        visRadius(n),
        Math.min(220, ctx.measureText(n.fullLabel).width / 2),
      ) + 8;
    const room = r.left - 14;
    if (sx + half <= room) return;
    const want = Math.max(room - half, insets.left + visRadius(n) + 4);
    if (want >= sx) return;
    viewT.x += want - sx;
    autoFit = false;
  }

  function applyPanelState() {
    details.classList.toggle("show", !!selected);
    details.classList.toggle("collapsed", panelCollapsed);
    $("dtoggle").setAttribute("aria-expanded", String(!panelCollapsed));
    $("dtoggle").setAttribute(
      "aria-label",
      panelCollapsed ? "Expand details" : "Collapse details",
    );
    measureInsets();
  }

  listen($("dtoggle"), "click", () => {
    panelCollapsed = !panelCollapsed;
    applyPanelState();
  });
  listen($("dclose"), "click", () => select(null, false));

  const { buildModel, renderModel } = createDetails({
    nodes,
    getEvidence: () => evidence,
    colors: COL,
    select,
  });

  function refreshDetails() {
    applyPanelState();
    const n = selected ? nodes.get(selected) : null;
    if (!n) {
      lastModelSig = null;
      lastDetailKey = null;
      dbody.replaceChildren();
      dtitle.textContent = "Details";
      return;
    }
    const model = buildModel(n);
    const sig = JSON.stringify(model);
    if (sig === lastModelSig && n.key === lastDetailKey) return;
    const keepScroll = n.key === lastDetailKey ? dbody.scrollTop : 0;
    lastModelSig = sig;
    lastDetailKey = n.key;
    dtitle.textContent =
      n.cls === "memory"
        ? "Memory: " + cut(n.d.text, 40)
        : n.cls === "fact"
          ? model.title + ": " + cut(n.d.name || n.d.statement, 40)
          : "Entity: " + cut(n.d.name, 40);
    dbody.replaceChildren(renderModel(model));
    dbody.scrollTop = keepScroll;
  }

  /* ---------- controls ---------- */
  const search = $("search");
  function updateMatchCount() {
    let c = 0;
    if (searchTerm)
      for (const n of nodeList)
        if (n.match && !n.dying && classVisible[n.cls]) c++;
    $("matches").textContent = searchTerm
      ? c + (c === 1 ? " match" : " matches")
      : "";
  }
  listen(search, "input", () => {
    searchTerm = search.value.trim().toLowerCase();
    for (const n of nodeList)
      n.match = !searchTerm || n.hay.includes(searchTerm);
    updateMatchCount();
    focusDirty = true;
    invalidate();
  });
  for (const cls of CLASSES) {
    listen($("t-" + cls), "click", () => {
      classVisible[cls] = !classVisible[cls];
      $("t-" + cls).setAttribute("aria-pressed", String(classVisible[cls]));
      applyVisibility();
      if (hover && !classVisible[hover.cls]) hover = null;
      updateMatchCount();
      reheat(0.3);
      focusDirty = true;
      invalidate();
    });
  }
  listen($("fit"), "click", () => {
    autoFit = true;
    fitView(false);
    invalidate();
  });
  listen(window, "keydown", (e) => {
    if (
      e.key === "/" &&
      document.activeElement !== search &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey
    ) {
      e.preventDefault();
      search.focus();
    } else if (e.key === "Escape") {
      if (document.activeElement === search || searchTerm) {
        search.value = "";
        search.dispatchEvent(new Event("input"));
        search.blur();
      } else if (selected) select(null, false);
    }
  });
  listen(window, "resize", resize);
  listen(document, "visibilitychange", () => {
    if (!document.hidden) invalidate();
  });
  if (window.ResizeObserver) {
    const ro = new ResizeObserver(() => {
      measureInsets();
      if (autoFit) fitView(false);
      invalidate();
    });
    ro.observe($("top"));
    ro.observe($("bottom"));
    ro.observe(details);
  }

  /* ---------- data loading ---------- */
  const ENDPOINTS = {
    projection: {
      label: "Neo4j projection",
      url: () =>
        "/api/v2/graph/projection?" +
        new URLSearchParams(
          LIMIT_PROJECTION === null
            ? { namespace: NS }
            : { namespace: NS, limit: LIMIT_PROJECTION },
        ),
    },
    memories: {
      label: "Postgres memories",
      url: () =>
        "/api/v2/graph/memories?" +
        new URLSearchParams(
          LIMIT_MEMORIES === null
            ? { namespace: NS }
            : { namespace: NS, limit: LIMIT_MEMORIES },
        ),
    },
  };

  function serverMessage(text) {
    try {
      const j = JSON.parse(text);
      if (j && typeof j.error === "string") return j.error;
      if (j && typeof j.message === "string") return j.message;
    } catch (_) {
      /* the body was not JSON, report the raw text below */
    }
    return cut(text, 300) || "empty response body";
  }

  async function fetchJson(name) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), REQUEST_TIMEOUT_MS);
    let res, text;
    try {
      res = await fetch(ENDPOINTS[name].url(), {
        cache: "no-store",
        signal: AbortSignal.any([ctl.signal, lifetime.signal]),
        headers: { accept: "application/json" },
      });
      text = await res.text();
    } catch (err) {
      if (err && err.name === "AbortError")
        throw new Error(
          "no response within " + REQUEST_TIMEOUT_MS / 1000 + " s",
        );
      throw new Error(
        "could not reach the server (" +
          (err && err.message ? err.message : err) +
          ")",
      );
    } finally {
      clearTimeout(timer);
    }
    if (!res.ok)
      throw new Error("HTTP " + res.status + ": " + serverMessage(text));
    let json;
    try {
      json = JSON.parse(text);
    } catch (_) {
      throw new Error("the response was not valid JSON");
    }
    return { text, json };
  }

  function validate(name, j) {
    if (!j || typeof j !== "object")
      throw new Error("unexpected response shape");
    if (
      name === "projection" &&
      !(Array.isArray(j.nodes) && Array.isArray(j.relationships))
    )
      throw new Error(
        "unexpected response shape: the nodes and relationships arrays are missing",
      );
    if (name === "memories" && !Array.isArray(j.memories))
      throw new Error(
        "unexpected response shape: the memories array is missing",
      );
    if (name === "memories") {
      const c = j.counts;
      if (!(
        c &&
        Number.isInteger(c.assertions) &&
        Number.isInteger(c.supported) &&
        c.assertions_by_status &&
        typeof c.assertions_by_status === "object" &&
        c.jobs &&
        typeof c.jobs === "object"
      )) {
        throw new Error(
          "unexpected response shape: counts.assertions, counts.supported, counts.assertions_by_status or counts.jobs is missing",
        );
      }
      if (
        !j.memories.every(
          (m) => m && m.extraction && typeof m.extraction.status === "string",
        )
      )
        throw new Error(
          "unexpected response shape: a memory has no extraction status",
        );
    }
  }

  async function cycle() {
    if (disposed) return;
    const names = ["projection", "memories"];
    const results = await Promise.allSettled(names.map((n) => fetchJson(n)));
    if (disposed) return;
    let dirty = false;
    results.forEach((r, i) => {
      const name = names[i];
      try {
        if (r.status === "rejected") throw r.reason;
        validate(name, r.value.json);
        state.errors[name] = null;
        state.okAt[name] = Date.now();
        if (r.value.text === state.lastText[name]) return;
        state.lastText[name] = r.value.text;
        state[name] = r.value.json;
        dirty = true;
      } catch (err) {
        state.errors[name] = {
          message: err && err.message ? err.message : String(err),
        };
      }
    });
    state.loaded = true;
    state.cycleOk = !names.some((n) => state.errors[n]);
    if (!state.errors.projection && !state.errors.memories)
      state.lastFullRefresh = Date.now();
    if (dirty) reconcile();
    renderErrors();
    renderStatus();
    renderMessage();
  }

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  /* nobody sees a hidden tab, so it does not poll; the next request goes out when it is shown again */
  function whenVisible() {
    if (!document.hidden) return Promise.resolve();
    return new Promise((resolve) => {
      const onChange = () => {
        if (document.hidden) return;
        document.removeEventListener("visibilitychange", onChange);
        resolve();
      };
      listen(document, "visibilitychange", onChange);
    });
  }
  async function pollLoop() {
    while (!disposed) {
      await whenVisible();
      if (disposed) return;
      try {
        await cycle();
      } catch (err) {
        console.error("graph refresh failed unexpectedly", err);
        state.errors.projection = {
          message:
            "internal error while refreshing: " +
            (err && err.message ? err.message : err),
        };
        renderErrors();
      }
      if (!POLL_MS) return;
      await sleep(POLL_MS);
    }
  }

  /* ---------- error banner, status strip, centered message ---------- */
  let lastErrSig = null;
  function renderErrors() {
    const box = $("err");
    const parts = [];
    for (const name of Object.keys(state.errors)) {
      if (state.errors[name])
        parts.push([ENDPOINTS[name].label, state.errors[name].message]);
    }
    const sig = parts.length
      ? JSON.stringify([parts, state.okAt, POLL_MS])
      : "ok";
    if (sig === lastErrSig) return;
    lastErrSig = sig;
    box.replaceChildren();
    if (!parts.length) {
      box.classList.remove("show");
      measureInsets();
      return;
    }
    for (const [label, message] of parts)
      box.append(
        h(
          "div",
          {},
          h("strong", { text: label + " request failed: " }),
          message,
        ),
      );
    const tail = [];
    for (const [name, label] of [
      ["projection", "Neo4j"],
      ["memories", "Postgres"],
    ]) {
      if (!state.errors[name]) continue;
      tail.push(
        state[name]
          ? "Showing earlier " +
              label +
              " data from " +
              fmtClock(state.okAt[name]) +
              ", it may be out of date."
          : "No " + label + " data has been loaded yet.",
      );
    }
    tail.push(
      POLL_MS
        ? "Retrying every " + POLL_MS / 1000 + " s."
        : "Live updates are off, reload the page to retry.",
    );
    box.append(h("div", { text: tail.join(" ") }));
    box.classList.add("show");
    measureInsets();
  }

  let lastStatusSig = null;
  const plural = (n, one, many) => n + " " + (n === 1 ? one : many);
  const sumCounts = (by) => Object.keys(by).reduce((t, k) => t + by[k], 0);
  const sameCounts = (a, b) => {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    for (const k of keys) if ((a[k] || 0) !== (b[k] || 0)) return false;
    return true;
  };
  const describeStatuses = (by) => {
    const keys = Object.keys(by)
      .filter((k) => by[k] > 0)
      .sort();
    return (
      plural(sumCounts(by), "assertion", "assertions") +
      (keys.length
        ? " (" + keys.map((k) => by[k] + " " + k).join(", ") + ")"
        : "")
    );
  };

  function renderStatus() {
    const parts = [];
    const p = state.projection,
      m = state.memories;
    if (p) {
      const c = p.counts || {};
      parts.push({
        cls: "",
        text:
          "Neo4j: " +
          (Number.isFinite(c.nodes) ? c.nodes : p.nodes.length) +
          " nodes, " +
          (Number.isFinite(c.relationships)
            ? c.relationships
            : p.relationships.length) +
          " relationships.",
      });
    } else {
      parts.push({
        cls: state.errors.projection ? "bad" : "",
        text: state.errors.projection
          ? "Neo4j: unavailable."
          : "Neo4j: loading.",
      });
    }
    if (m) {
      const c = m.counts;
      parts.push({
        cls: "",
        text:
          "Postgres: " +
          (Number.isFinite(c.memories) ? c.memories : m.memories.length) +
          " memories (" +
          c.supported +
          " supporting facts).",
      });
      const queued = c.jobs.extract_pending + c.jobs.extract_running;
      if (queued)
        parts.push({
          cls: "",
          text:
            "Extraction: " +
            c.jobs.extract_pending +
            " queued, " +
            c.jobs.extract_running +
            " running.",
        });
    } else {
      parts.push({
        cls: state.errors.memories ? "bad" : "",
        text: state.errors.memories
          ? "Postgres: unavailable."
          : "Postgres: loading.",
      });
    }
    const mode = POLL_MS
      ? "live, every " + POLL_MS / 1000 + " s"
      : "live updates off";
    parts.push({
      cls: "live",
      text: state.lastFullRefresh
        ? "Updated " + fmtClock(state.lastFullRefresh) + ", " + mode
        : state.loaded
          ? "Not refreshed, " + mode
          : "Loading",
      color:
        state.cycleOk === false
          ? "var(--red)"
          : !POLL_MS
            ? "var(--faint)"
            : "var(--memory)",
    });

    const notes = pageNotes.slice();
    if (p && p.truncated)
      notes.push(
        "Neo4j result truncated at " +
          p.nodes.length +
          " nodes, the graph is incomplete" +
          (LIMIT_PROJECTION === null
            ? ". Add &limit=2000 to the page URL to raise the limit."
            : "."),
      );
    if (m && m.truncated)
      notes.push(
        "Postgres memories truncated at " +
          m.memories.length +
          " of the namespace. Memories that no fact cites yet and the most recent ones are kept first, the rest are not shown" +
          (LIMIT_MEMORIES === null
            ? ". Add &memory_limit=1000 to the page URL to raise the limit."
            : "."),
      );
    if (p && m && !p.truncated) {
      const neo = {};
      for (const n of p.nodes)
        if (n.kind === "assertion") {
          const st = n.status || "unknown";
          neo[st] = (neo[st] || 0) + 1;
        }
      const pg = m.counts.assertions_by_status;
      if (!sameCounts(neo, pg))
        notes.push(
          (sumCounts(neo) < sumCounts(pg)
            ? "Projection behind: "
            : "Projection out of step: ") +
            "Neo4j has " +
            describeStatuses(neo) +
            " but Postgres has " +
            describeStatuses(pg) +
            ".",
        );
    }
    if (m) {
      const j = m.counts.jobs;
      if (j.extract_failed) {
        notes.push(
          plural(j.extract_failed, "extraction job", "extraction jobs") +
            " failed" +
            (j.extract_blocked
              ? " and " +
                plural(j.extract_blocked, "queued job is", "queued jobs are") +
                " blocked behind " +
                (j.extract_failed === 1 ? "it" : "them")
              : "") +
            ". Their memories get no facts until the failed " +
            (j.extract_failed === 1 ? "job is" : "jobs are") +
            " retried.",
        );
      }
      if (j.project_failed)
        notes.push(
          plural(j.project_failed, "projection job", "projection jobs") +
            " failed, so Neo4j stays behind Postgres until " +
            (j.project_failed === 1 ? "it is" : "they are") +
            " retried.",
        );
      else if (j.project_pending + j.project_running)
        notes.push(
          "Projection catching up: " +
            plural(j.project_pending + j.project_running, "job", "jobs") +
            " still to run, Neo4j may be behind Postgres.",
        );
    }
    if (info.danglingSupports)
      notes.push(
        info.danglingSupports +
          (info.danglingSupports === 1
            ? " evidence link points"
            : " evidence links point") +
          " to a fact that is missing from the Neo4j projection, so it is not drawn.",
      );
    if (info.unknownNodes)
      notes.push(
        info.unknownNodes +
          (info.unknownNodes === 1 ? " node" : " nodes") +
          " with an unknown kind or id ignored.",
      );
    if (info.badRelationships)
      notes.push(
        info.badRelationships +
          (info.badRelationships === 1 ? " relationship" : " relationships") +
          " with an unknown type or a missing endpoint ignored.",
      );
    for (const note of notes) parts.push({ cls: "amber", text: note });

    const sig = JSON.stringify(parts);
    if (sig === lastStatusSig) return;
    lastStatusSig = sig;
    $("strip").replaceChildren(
      ...parts.map((pt) =>
        h("span", {
          class: pt.cls,
          text: pt.text,
          style: pt.color ? "--c:" + pt.color : null,
        }),
      ),
    );
    measureInsets();
  }

  let lastMsgSig = null;
  function renderMessage() {
    let body = null;
    if (!state.loaded) body = "Loading the graph for " + NS + ".";
    else if (
      !state.errors.projection &&
      !state.errors.memories &&
      !nodeList.some((n) => !n.dying)
    )
      body = "No memories yet for " + NS + "." + (POLL_MS ? " Waiting." : "");
    if (body === lastMsgSig) return;
    lastMsgSig = body;
    if (!body) {
      $("msg").classList.remove("show");
      return;
    }
    $("msgtext").replaceChildren(body);
    $("msg").classList.add("show");
  }

  /* ---------- clear the namespace ---------- */
  /* Asks for the namespace to be typed, then deletes everything it knows (POST /api/v2/graph/clear). The next
     refresh draws the empty graph. */
  async function clearNamespace() {
    if (!hasNs) return;
    const typed = window.prompt(
      'This deletes every fact, entity, memory and source text of the namespace "' +
        NS +
        '" and empties its graph. It cannot be undone.\n\nType the namespace to confirm:',
    );
    if (typed === null) return;
    if (typed !== NS) {
      window.alert("Not cleared: what you typed does not match the namespace.");
      return;
    }
    const button = $("clear");
    button.disabled = true;
    try {
      const res = await fetch("/api/v2/graph/clear", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          accept: "application/json",
        },
        body: JSON.stringify({ namespace: NS, confirm: typed }),
      });
      const text = await res.text();
      if (!res.ok)
        throw new Error("HTTP " + res.status + ": " + serverMessage(text));
      const done = JSON.parse(text);
      if (done.graph && done.graph.cleared === false)
        window.alert(
          "The memory was cleared, but Neo4j was not reached yet (" +
            (done.graph.reason || "unknown reason") +
            "). The queued clear job will try again.",
        );
      await cycle();
    } catch (err) {
      window.alert("Not cleared: " + (err && err.message ? err.message : err));
    } finally {
      button.disabled = false;
    }
  }
  listen($("clear"), "click", clearNamespace);

  /* ---------- start ---------- */
  function start() {
    $("nsname").textContent = hasNs ? NS : "none";
    $("clear").hidden = !hasNs;
    if (hasNs) $("nschip").title = NS;
    document.title = hasNs ? "Memory Graph: " + NS : "Memory Graph";
    resize();
    view.x = viewT.x = W / 2;
    view.y = viewT.y = H / 2;
    if (!hasNs) {
      $("msgtext").replaceChildren(
        h("strong", { text: "Namespace required" }),
        "Open this page with a namespace, for example /graph?namespace=user:haz3",
      );
      $("msg").classList.add("show");
      $("strip").replaceChildren(
        h("span", {
          class: "amber",
          text: "No namespace in the URL, nothing is being loaded.",
        }),
      );
      return;
    }
    renderStatus();
    renderMessage();
    pollLoop();
  }
  start();
  return () => {
    disposed = true;
    lifetime.abort();
    cancelAnimationFrame(raf);
    clearTimeout(settleTimer);
    cleanups.forEach((cleanup) => cleanup());
  };
}
