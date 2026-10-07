import { useState } from "react";
import Button from "react-bootstrap/Button";
import Form from "react-bootstrap/Form";
import {
  directories,
  directoryFrom,
  graphUrl,
  terminalUrl,
  type Directory,
} from "./session";
import "./demo.css";

export function DemoPage() {
  const query = new URLSearchParams(location.search);
  const [namespace, setNamespace] = useState(
    query.get("namespace")?.trim() || "user:dev",
  );
  const [draft, setDraft] = useState(namespace);
  const [directory, setDirectory] = useState<Directory>(
    directoryFrom(query.get("directory")),
  );
  const [session, setSession] = useState(0);

  function open(nextDirectory: Directory, nextNamespace = namespace) {
    setDirectory(nextDirectory);
    setNamespace(nextNamespace);
    setSession((value) => value + 1);
    history.replaceState(
      null,
      "",
      `/demo?${new URLSearchParams({ namespace: nextNamespace, directory: nextDirectory })}`,
    );
  }

  return (
    <div className="pi-demo">
      <header className="demo-header">
        <div className="demo-intro">
          <h1>Pi with shared memory</h1>
          <p>
            Tell Pi a fact, open a session in another directory, and ask about
            it. The graph updates as memories are processed.
          </p>
        </div>
        <div className="demo-sessions" aria-label="Session directories">
          {directories.map((item) => (
            <Button
              key={item}
              id={`b-${item}`}
              variant={directory === item ? "primary" : "outline-light"}
              aria-pressed={directory === item}
              onClick={() => open(item)}
            >
              Session in {item}
            </Button>
          ))}
        </div>
        <Form
          className="demo-namespace"
          onSubmit={(event) => {
            event.preventDefault();
            const next = draft.trim();
            if (next && next !== namespace) open(directory, next);
          }}
        >
          <Form.Label htmlFor="demo-namespace">Namespace</Form.Label>
          <Form.Control
            id="demo-namespace"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            required
          />
          <Button
            type="submit"
            variant="outline-light"
            disabled={!draft.trim() || draft.trim() === namespace}
          >
            Apply
          </Button>
          <span className="demo-help">
            Changing the namespace starts a fresh Pi session.
          </span>
        </Form>
      </header>
      <main className="demo-panes">
        <section className="demo-pane" aria-labelledby="term-title">
          <div className="demo-pane-heading">
            <h2 id="term-title">Pi, session in {directory}</h2>
            <Button
              variant="outline-light"
              size="sm"
              onClick={() => open(directory)}
            >
              New session
            </Button>
          </div>
          <iframe
            id="term"
            title="Pi terminal"
            allow="clipboard-write"
            src={terminalUrl(directory, namespace, session)}
          />
        </section>
        <section className="demo-pane" aria-labelledby="graph-title">
          <div className="demo-pane-heading">
            <h2 id="graph-title">Live memory graph · {namespace}</h2>
            <a
              href={graphUrl(namespace)}
              target="_blank"
              rel="noopener noreferrer"
            >
              Open graph
            </a>
          </div>
          <iframe
            key={namespace}
            id="graph"
            title="Memory graph"
            src={graphUrl(namespace)}
          />
        </section>
      </main>
    </div>
  );
}
