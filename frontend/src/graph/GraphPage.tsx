import { useEffect, useRef } from "react";
import Button from "react-bootstrap/Button";
import { mountGraph } from "./viewer.js";
import "./graph.css";

const layers = ["memory", "fact", "entity"] as const;
const layerLabels = { memory: "Memories", fact: "Facts", entity: "Entities" };

// React owns the controls; the canvas viewer owns layout, drawing, and pointer events.
export function GraphPage() {
  const container = useRef<HTMLDivElement>(null);
  useEffect(() => {
    document.body.classList.add("graph-mode");
    const cleanup = mountGraph(container.current!);
    return () => {
      cleanup();
      document.body.classList.remove("graph-mode");
    };
  }, []);

  return (
    <div ref={container}>
      <canvas id="cv" tabIndex={0} role="img" aria-label="Memory graph" />
      <div id="ui">
        <div id="top">
          <div id="bar">
            <Button
              as="a"
              href="/"
              size="sm"
              variant="outline-light"
              className="pill"
            >
              Workspace
            </Button>
            <span className="pill" id="nschip">
              Namespace <b id="nsname" />
            </span>
            <input
              className="pill"
              id="search"
              type="search"
              placeholder="Search nodes"
              aria-label="Search nodes"
              autoComplete="off"
              spellCheck={false}
            />
            <span id="matches" aria-live="polite" />
            {layers.map((layer) => (
              <Button
                key={layer}
                variant="outline-light"
                className="pill chip"
                id={`t-${layer}`}
                aria-pressed="true"
                style={{ "--c": `var(--${layer})` } as React.CSSProperties}
              >
                <span className="dot" />
                {layerLabels[layer]} <b id={`c-${layer}`}>0</b>
              </Button>
            ))}
            <Button variant="outline-light" className="pill" id="fit">
              Fit
            </Button>
            <Button
              variant="outline-danger"
              className="pill"
              id="clear"
              title="Delete everything this namespace knows"
            >
              Clear
            </Button>
          </div>
          <div id="err" role="alert" />
        </div>
        <div id="mid">
          <aside id="details" aria-label="Node details">
            <div id="dhead">
              <span id="dtitle">Details</span>
              <button
                id="dtoggle"
                type="button"
                aria-expanded="true"
                aria-label="Collapse details"
              >
                −
              </button>
              <button id="dclose" type="button" aria-label="Close details">
                ×
              </button>
            </div>
            <div id="dbody" />
          </aside>
        </div>
        <div id="bottom">
          <details id="legend" open>
            <summary>Legend</summary>
            <ul>
              <li>
                <span className="legend-dot memory" />
                Memory: what was said
              </li>
              <li>
                <span className="legend-dot fact" />
                Fact: extracted from memories
              </li>
              <li>
                <span className="legend-dot observation" />
                Observation: derived fact
              </li>
              <li>
                <span className="legend-dot entity" />
                Entity: what facts mention
              </li>
              <li>Faded facts are superseded, retracted, stale, or expired.</li>
              <li>Amber rings mark failed or blocked extraction.</li>
              <li>Green links show evidence; gray links connect entities.</li>
              <li>Purple arrows connect facts. Hover to see the relation.</li>
            </ul>
            <div className="hint">
              Scroll to zoom, drag the background to pan. Drag a node to pin it,
              double click to release.
            </div>
          </details>
          <div id="strip" role="status" aria-live="polite" />
        </div>
      </div>
      <div id="msg" role="status">
        <div id="msgtext" />
      </div>
      <div id="tip" />
    </div>
  );
}
