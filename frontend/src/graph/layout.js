// Force layout has no DOM dependencies. Positions are updated in place.
export function createLayout(sim) {
  const ALPHA_MIN = 0.001;
  const ALPHA_DECAY = 1 - Math.pow(ALPHA_MIN, 1 / 300);
  const VELOCITY_DECAY = 0.6;
  const THETA2 = 0.81;
  const MAX_DIST2 = 700 * 700;
  const MIN_DIST2 = 36;
  const GRAVITY = 0.14;

  /* ---------- force simulation: Barnes-Hut charge, link springs, gravity, collision ---------- */
  let cap = 0,
    px,
    py,
    pw,
    qCap = 0;
  let qx0, qy0, qs, qm, qmx, qmy, qn, qpt, qleaf, qk, qStack;
  let qCount = 0;
  let hashNext, hashGx, hashGy;
  const HASH_SIZE = 1 << 14;
  const hashHead = new Int32Array(HASH_SIZE);

  function ensureCap(n) {
    if (n <= cap) return;
    cap = Math.max(256, Math.ceil(n * 1.5));
    px = new Float64Array(cap);
    py = new Float64Array(cap);
    pw = new Float64Array(cap);
    hashNext = new Int32Array(cap);
    hashGx = new Int32Array(cap);
    hashGy = new Int32Array(cap);
    growQuad(cap * 3 + 32);
  }

  function growQuad(c) {
    const f64 = (old) => {
      const a = new Float64Array(c);
      if (old) a.set(old);
      return a;
    };
    const i32 = (old, mul) => {
      const a = new Int32Array(c * mul);
      if (old) a.set(old);
      return a;
    };
    qx0 = f64(qx0);
    qy0 = f64(qy0);
    qs = f64(qs);
    qm = f64(qm);
    qmx = f64(qmx);
    qmy = f64(qmy);
    qn = i32(qn, 1);
    qpt = i32(qpt, 1);
    qk = i32(qk, 4);
    const lf = new Uint8Array(c);
    if (qleaf) lf.set(qleaf);
    qleaf = lf;
    qStack = new Int32Array(Math.max(256, Math.ceil(c / 2)));
    qCap = c;
  }

  function newCell(x0, y0, s) {
    if (qCount >= qCap) growQuad(qCap * 2);
    const c = qCount++;
    qx0[c] = x0;
    qy0[c] = y0;
    qs[c] = s;
    qm[c] = 0;
    qmx[c] = 0;
    qmy[c] = 0;
    qn[c] = 0;
    qpt[c] = -1;
    qleaf[c] = 1;
    const b = c * 4;
    qk[b] = -1;
    qk[b + 1] = -1;
    qk[b + 2] = -1;
    qk[b + 3] = -1;
    return c;
  }

  function quadInsert(i) {
    const x = px[i],
      y = py[i],
      w = pw[i];
    let c = 0,
      depth = 0;
    for (;;) {
      qm[c] += w;
      qmx[c] += x * w;
      qmy[c] += y * w;
      qn[c]++;
      if (qleaf[c]) {
        if (qn[c] === 1) {
          qpt[c] = i;
          return;
        }
        const p = qpt[c];
        if (depth >= 22 || p < 0 || (px[p] === x && py[p] === y)) {
          qpt[c] = -2;
          return;
        }
        qleaf[c] = 0;
        qpt[c] = -1;
        const half = qs[c] / 2;
        const jp =
          (px[p] >= qx0[c] + half ? 1 : 0) | (py[p] >= qy0[c] + half ? 2 : 0);
        const child = newCell(
          qx0[c] + (jp & 1 ? half : 0),
          qy0[c] + (jp & 2 ? half : 0),
          half,
        );
        qk[c * 4 + jp] = child;
        qm[child] = pw[p];
        qmx[child] = px[p] * pw[p];
        qmy[child] = py[p] * pw[p];
        qn[child] = 1;
        qpt[child] = p;
      }
      const half = qs[c] / 2;
      const j = (x >= qx0[c] + half ? 1 : 0) | (y >= qy0[c] + half ? 2 : 0);
      let child = qk[c * 4 + j];
      if (child < 0) {
        child = newCell(
          qx0[c] + (j & 1 ? half : 0),
          qy0[c] + (j & 2 ? half : 0),
          half,
        );
        qk[c * 4 + j] = child;
      }
      c = child;
      depth++;
    }
  }

  function buildQuad(n) {
    let minX = Infinity,
      minY = Infinity,
      maxX = -Infinity,
      maxY = -Infinity;
    for (let i = 0; i < n; i++) {
      const x = px[i],
        y = py[i];
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }
    qCount = 0;
    newCell(minX - 1, minY - 1, Math.max(maxX - minX, maxY - minY) + 2);
    for (let i = 0; i < n; i++) quadInsert(i);
  }

  function repel(i, alpha, simNodes) {
    const x = px[i],
      y = py[i];
    let fx = 0,
      fy = 0,
      sp = 0;
    qStack[sp++] = 0;
    while (sp > 0) {
      const c = qStack[--sp];
      const m = qm[c];
      if (m === 0) continue;
      let dx = qmx[c] / m - x,
        dy = qmy[c] / m - y;
      let d2 = dx * dx + dy * dy;
      if (qleaf[c]) {
        if (qpt[c] === i) continue;
        if (d2 === 0) {
          dx = (Math.random() - 0.5) * 0.1;
          dy = (Math.random() - 0.5) * 0.1;
          d2 = dx * dx + dy * dy + 1e-6;
        }
      } else if (qs[c] * qs[c] >= THETA2 * d2) {
        const b = c * 4;
        for (let j = 0; j < 4; j++) {
          const k = qk[b + j];
          if (k >= 0) qStack[sp++] = k;
        }
        continue;
      }
      if (d2 > MAX_DIST2) continue;
      if (d2 < MIN_DIST2) d2 = MIN_DIST2;
      const w = (m * alpha) / d2;
      fx -= dx * w;
      fy -= dy * w;
    }
    const nd = simNodes[i];
    nd.vx += fx;
    nd.vy += fy;
  }

  function collide(simNodes) {
    const n = simNodes.length;
    const cell = 46;
    hashHead.fill(-1);
    for (let i = 0; i < n; i++) {
      const nd = simNodes[i];
      const gx = Math.floor(nd.x / cell),
        gy = Math.floor(nd.y / cell);
      hashGx[i] = gx;
      hashGy[i] = gy;
      const hs = ((gx * 73856093) ^ (gy * 19349663)) & (HASH_SIZE - 1);
      hashNext[i] = hashHead[hs];
      hashHead[hs] = i;
    }
    for (let i = 0; i < n; i++) {
      const a = simNodes[i];
      const ra = a.r + 6;
      const gx = hashGx[i],
        gy = hashGy[i];
      for (let ox = -1; ox <= 1; ox++) {
        for (let oy = -1; oy <= 1; oy++) {
          const cx = gx + ox,
            cy = gy + oy;
          let j =
            hashHead[((cx * 73856093) ^ (cy * 19349663)) & (HASH_SIZE - 1)];
          for (; j >= 0; j = hashNext[j]) {
            if (j <= i || hashGx[j] !== cx || hashGy[j] !== cy) continue;
            const b = simNodes[j];
            const rr = ra + b.r + 6;
            let dx = a.x + a.vx - b.x - b.vx,
              dy = a.y + a.vy - b.y - b.vy;
            let l = dx * dx + dy * dy;
            if (l >= rr * rr) continue;
            if (l === 0) {
              dx = (Math.random() - 0.5) * 0.1;
              dy = (Math.random() - 0.5) * 0.1;
              l = dx * dx + dy * dy + 1e-6;
            }
            l = Math.sqrt(l);
            const f = ((rr - l) / l) * 0.7;
            dx *= f;
            dy *= f;
            const ra2 = a.r * a.r,
              rb2 = b.r * b.r;
            const share = rb2 / (ra2 + rb2);
            a.vx += dx * share;
            a.vy += dy * share;
            b.vx -= dx * (1 - share);
            b.vy -= dy * (1 - share);
          }
        }
      }
    }
  }

  function tick(simNodes, simLinks) {
    const n = simNodes.length;
    if (!n) {
      sim.active = false;
      return;
    }
    sim.alpha += (sim.alphaTarget - sim.alpha) * ALPHA_DECAY;
    const alpha = sim.alpha;
    ensureCap(n);
    for (let i = 0; i < n; i++) {
      const nd = simNodes[i];
      px[i] = nd.x;
      py[i] = nd.y;
      pw[i] = nd.q;
    }

    for (let i = 0, m = simLinks.length; i < m; i++) {
      const l = simLinks[i],
        s = l.s,
        t = l.t;
      let dx = t.x + t.vx - s.x - s.vx,
        dy = t.y + t.vy - s.y - s.vy;
      if (dx === 0 && dy === 0) {
        dx = (Math.random() - 0.5) * 0.1;
        dy = (Math.random() - 0.5) * 0.1;
      }
      const len = Math.sqrt(dx * dx + dy * dy);
      const f = ((len - l.dist * sim.spread) / len) * alpha * l.strength;
      dx *= f;
      dy *= f;
      t.vx -= dx * l.bias;
      t.vy -= dy * l.bias;
      s.vx += dx * (1 - l.bias);
      s.vy += dy * (1 - l.bias);
    }

    buildQuad(n);
    for (let i = 0; i < n; i++) repel(i, alpha, simNodes);

    const g = GRAVITY * alpha;
    for (let i = 0; i < n; i++) {
      const nd = simNodes[i];
      nd.vx -= nd.x * g;
      nd.vy -= nd.y * g;
    }

    collide(simNodes);

    for (let i = 0; i < n; i++) {
      const nd = simNodes[i];
      if (nd.fx !== null) {
        nd.x = nd.fx;
        nd.y = nd.fy;
        nd.vx = 0;
        nd.vy = 0;
        continue;
      }
      nd.vx *= VELOCITY_DECAY;
      nd.vy *= VELOCITY_DECAY;
      nd.x += nd.vx;
      nd.y += nd.vy;
      if (!(nd.x === nd.x && nd.y === nd.y)) {
        if (sim.nanResets++ === 0)
          console.error(
            "graph layout produced a non-finite position, resetting that node",
          );
        nd.x = (Math.random() - 0.5) * 20;
        nd.y = (Math.random() - 0.5) * 20;
        nd.vx = 0;
        nd.vy = 0;
      }
    }
    if (sim.alpha < ALPHA_MIN && sim.alphaTarget === 0) sim.active = false;
  }

  return { tick };
}
