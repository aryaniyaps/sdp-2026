import { h, fmtTime, shortId, cut } from "./helpers.js";

// Build a detail model separately from its DOM representation.
export function createDetails({ nodes, getEvidence, colors: COL, select }) {
  function summary(n) {
    const label =
      n.cls === "memory"
        ? cut(n.d.text, 90)
        : n.cls === "fact"
          ? cut(n.d.statement || n.d.name, 90)
          : cut(n.d.name, 90);
    return {
      key: n.key,
      cls: n.cls,
      label,
      dim: n.dimStatus,
      observation: n.observation,
      status: n.cls === "fact" ? n.d.status || null : null,
    };
  }

  function buildModel(n) {
    const d = n.d;
    if (n.cls === "memory") {
      /* one entry per fact: the server sends one row per quote */
      const byFact = new Map();
      for (const s of d.supports || []) {
        let g = byFact.get(s.assertion_id);
        if (!g) {
          const f = nodes.get("f:" + s.assertion_id);
          g = {
            fact: f && !f.dying ? summary(f) : null,
            assertionId: s.assertion_id,
            quotes: [],
          };
          byFact.set(s.assertion_id, g);
        }
        g.quotes.push(s.quote);
      }
      return {
        cls: "memory",
        title: "Memory",
        role: d.role || null,
        when: d.occurred_at || null,
        state: d.processing_state === undefined ? null : d.processing_state,
        extraction: d.extraction || null,
        session: d.session_id || null,
        id: d.id,
        text: d.text,
        truncated: !!d.text_truncated,
        supports: Array.from(byFact.values()),
      };
    }
    if (n.cls === "fact") {
      const ev = Array.from(
        getEvidence().get(d.id) || [],
        ([memoryKey, quotes]) => {
          const m = nodes.get(memoryKey);
          return {
            memory:
              m && !m.dying
                ? {
                    key: m.key,
                    role: m.d.role,
                    when: m.d.occurred_at,
                    text: cut(m.d.text, 120),
                  }
                : null,
            quotes,
          };
        },
      );
      const ents = [],
        rels = [];
      for (const l of n.adj) {
        const o = l.s === n ? l.t : l.s;
        if (l.type === "mention" && l.s === n) ents.push(summary(o));
        else if (l.type === "rel")
          rels.push({
            other: summary(o),
            relation: l.relation,
            explanation: l.explanation,
            outgoing: l.s === n,
          });
      }
      return {
        cls: "fact",
        title: n.observation ? "Observation" : "Fact",
        statement: d.statement,
        status: d.status,
        kind: d.assertion_kind,
        validFrom: d.valid_from,
        validTo: d.valid_to,
        revision: d.revision,
        expired: n.validToMs !== null && n.validToMs <= Date.now(),
        id: d.id,
        evidence: ev,
        entities: ents,
        relations: rels,
        observation: n.observation,
        dim: n.dimStatus,
        name: d.name,
      };
    }
    const facts = [];
    for (const l of n.adj) if (l.type === "mention") facts.push(summary(l.s));
    return {
      cls: "entity",
      title: "Entity",
      name: d.name,
      type: d.entity_type,
      id: d.id,
      facts,
    };
  }

  function dot(s) {
    return h("i", {
      class: (s.observation ? "diamond " : "") + (s.dim ? "dimmed" : ""),
      style: "--c:" + COL[s.cls],
    });
  }

  function nodeRow(s, sub) {
    return h(
      "button",
      { class: "row", type: "button", onclick: () => select(s.key, true) },
      h("div", { class: "rhead" }, dot(s), h("span", { text: s.label })),
      sub ? h("div", { class: "rsub", text: sub }) : null,
    );
  }

  function kv(pairs) {
    const dl = h("dl", { class: "kv" });
    for (const [k, v, mono] of pairs)
      dl.append(
        h("dt", { text: k }),
        h("dd", { class: mono ? "mono" : "", text: v }),
      );
    return dl;
  }

  const EXTRACTION_WORD = {
    pending: "queued",
    running: "running",
    failed: "failed",
    blocked: "blocked",
  };

  /* what the extract job of a memory did, in words; the server error text is shown as sent */
  function extractionText(x) {
    if (!x) return "unknown";
    const tries =
      x.attempts !== null && x.attempts !== undefined && x.max_attempts
        ? " (attempt " + x.attempts + " of " + x.max_attempts + ")"
        : "";
    switch (x.status) {
      case "succeeded":
        return "succeeded";
      case "none":
        return "no extraction job exists for this memory";
      case "running":
        return "running" + tries;
      case "pending":
        return x.error
          ? "queued for a retry" + tries + ", last error: " + x.error
          : "queued";
      case "failed":
        return (
          "failed after " +
          x.attempts +
          " of " +
          x.max_attempts +
          " attempts, last error: " +
          (x.error || "none recorded")
        );
      case "blocked":
        return (
          "blocked behind a failed extraction of this namespace" +
          (x.error ? " (" + x.error + ")" : "") +
          ". The worker does not skip it, so this memory waits until that job is retried."
        );
      default:
        return String(x.status);
    }
  }

  /* why a memory has no facts, as a note for the details panel */
  function noFactsNote(m) {
    const x = m.extraction;
    const status = x ? x.status : null;
    if (status === "failed")
      return h("div", {
        class: "note bad",
        text: "No facts: extraction failed" + (x.error ? ". " + x.error : "."),
      });
    if (status === "blocked")
      return h("div", {
        class: "note",
        text: "No facts yet: extraction is blocked behind a failed extraction in this namespace.",
      });
    if (status === "running")
      return h("div", { class: "empty", text: "Extraction is running." });
    if (status === "pending")
      return h("div", {
        class: "empty",
        text: "Not extracted yet, it is queued.",
      });
    if (m.state === "failed")
      return h("div", {
        class: "note bad",
        text: "Extraction failed for this memory.",
      });
    if (m.state === "pending")
      return h("div", { class: "empty", text: "Not extracted yet." });
    return h("div", { class: "empty", text: "No fact cites this memory." });
  }

  function renderModel(m) {
    const frag = document.createDocumentFragment();
    frag.append(
      h(
        "div",
        {
          class: "kindpill" + (m.observation ? " diamond" : ""),
          style: "--c:" + COL[m.cls],
        },
        h("i"),
        m.title,
      ),
    );
    if (m.cls === "memory") {
      const badges = h("div", { class: "badges" });
      if (m.role) badges.append(h("span", { class: "badge", text: m.role }));
      badges.append(
        h("span", {
          class:
            "badge " +
            (m.state === "processed"
              ? "good"
              : m.state === "failed"
                ? "warn"
                : ""),
          text: m.state === null ? "processing state unknown" : m.state,
        }),
      );
      const xs = m.extraction ? m.extraction.status : null;
      if (EXTRACTION_WORD[xs])
        badges.append(
          h("span", {
            class:
              "badge " +
              (xs === "failed" ? "bad" : xs === "blocked" ? "warn" : ""),
            text: "extraction " + EXTRACTION_WORD[xs],
          }),
        );
      frag.append(badges);
      frag.append(
        kv([
          ["When", fmtTime(m.when) || "unknown"],
          ["Session", m.session ? shortId(m.session) : "none", true],
          ["Id", shortId(m.id), true],
          ["Extraction", extractionText(m.extraction)],
        ]),
      );
      frag.append(
        h("h4", { text: "Text" }),
        h("div", { class: "fulltext", text: m.text }),
      );
      if (m.truncated)
        frag.append(
          h("div", {
            class: "note",
            text: "The server truncated this text to 600 characters.",
          }),
        );
      frag.append(
        h("h4", { text: "Facts it supports (" + m.supports.length + ")" }),
      );
      if (!m.supports.length) {
        frag.append(noFactsNote(m));
      } else {
        const list = h("div", { class: "list" });
        for (const s of m.supports) {
          const quotes = s.quotes.map((q) =>
            h("div", { class: "quote", text: q }),
          );
          if (s.fact) {
            const row = nodeRow(s.fact);
            row.append(...quotes);
            list.append(row);
          } else {
            list.append(
              h(
                "div",
                { class: "row" },
                h("span", {
                  text:
                    "Fact " +
                    shortId(s.assertionId) +
                    " is not in the Neo4j projection",
                }),
                ...quotes,
              ),
            );
          }
        }
        frag.append(list);
      }
    } else if (m.cls === "fact") {
      frag.append(
        h("div", { class: "statement", text: m.statement || m.name || "" }),
      );
      const badges = h("div", { class: "badges" });
      badges.append(
        h("span", {
          class:
            "badge " + (m.status === "active" ? "good" : m.dim ? "warn" : ""),
          text: m.status || "status unknown",
        }),
      );
      if (m.kind) badges.append(h("span", { class: "badge", text: m.kind }));
      if (m.expired && m.status !== "superseded")
        badges.append(h("span", { class: "badge warn", text: "expired" }));
      frag.append(badges);
      frag.append(
        kv([
          ["Status", m.status || "unknown"],
          ["Kind", m.kind || "unknown"],
          ["Valid from", fmtTime(m.validFrom) || "not set"],
          ["Valid to", fmtTime(m.validTo) || "none (still valid)"],
          [
            "Revision",
            m.revision === null || m.revision === undefined
              ? "unknown"
              : String(m.revision),
          ],
          ["Id", shortId(m.id), true],
        ]),
      );
      frag.append(h("h4", { text: "Evidence (" + m.evidence.length + ")" }));
      if (!m.evidence.length)
        frag.append(
          h("div", {
            class: "empty",
            text: "No loaded memory cites this fact.",
          }),
        );
      else {
        const list = h("div", { class: "list" });
        for (const e of m.evidence) {
          const quotes = e.quotes.map((q) =>
            h("div", { class: "quote", text: q }),
          );
          if (e.memory) {
            list.append(
              h(
                "button",
                {
                  class: "row",
                  type: "button",
                  onclick: () => select(e.memory.key, true),
                },
                h(
                  "div",
                  { class: "rhead" },
                  h("i", { style: "--c:" + COL.memory }),
                  h("span", {
                    text:
                      (e.memory.role || "memory") +
                      ", " +
                      (fmtTime(e.memory.when) || "unknown time"),
                  }),
                ),
                h("div", { class: "rsub", text: e.memory.text }),
                ...quotes,
              ),
            );
          } else
            list.append(
              h(
                "div",
                { class: "row" },
                h("span", { text: "Memory not loaded" }),
                ...quotes,
              ),
            );
        }
        frag.append(list);
      }
      frag.append(h("h4", { text: "Mentions (" + m.entities.length + ")" }));
      if (!m.entities.length)
        frag.append(h("div", { class: "empty", text: "No entities." }));
      else {
        const list = h("div", { class: "list" });
        for (const s of m.entities) list.append(nodeRow(s));
        frag.append(list);
      }
      if (m.relations.length) {
        frag.append(
          h("h4", { text: "Relations (" + m.relations.length + ")" }),
        );
        const list = h("div", { class: "list" });
        for (const r of m.relations) {
          const rel = h("span", {
            class: "rel",
            text: r.relation || "relates to",
          });
          const line = r.outgoing
            ? h("span", {}, "This fact ", rel, " " + r.other.label)
            : h("span", {}, r.other.label + " ", rel, " this fact");
          const row = h(
            "button",
            {
              class: "row",
              type: "button",
              onclick: () => select(r.other.key, true),
            },
            h("div", { class: "rhead" }, dot(r.other), line),
          );
          if (r.explanation)
            row.append(h("div", { class: "rsub", text: r.explanation }));
          list.append(row);
        }
        frag.append(list);
      }
    } else {
      frag.append(h("div", { class: "statement", text: m.name || "" }));
      if (m.type)
        frag.append(
          h(
            "div",
            { class: "badges" },
            h("span", { class: "badge", text: m.type }),
          ),
        );
      frag.append(
        kv([
          ["Type", m.type || "unknown"],
          ["Id", shortId(m.id), true],
        ]),
      );
      frag.append(
        h("h4", {
          text:
            "Mentioned by " +
            m.facts.length +
            (m.facts.length === 1 ? " fact" : " facts"),
        }),
      );
      if (!m.facts.length)
        frag.append(
          h("div", { class: "empty", text: "No fact mentions this entity." }),
        );
      else {
        const list = h("div", { class: "list" });
        for (const s of m.facts) list.append(nodeRow(s, s.status));
        frag.append(list);
      }
    }
    return frag;
  }

  return { buildModel, renderModel };
}
