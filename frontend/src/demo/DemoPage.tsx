import { useEffect, useRef, useState } from "react";
import Button from "react-bootstrap/Button";
import Form from "react-bootstrap/Form";
import Modal from "react-bootstrap/Modal";
import {
  dashboardRequest,
  directoryFrom,
  graphUrl,
  sampleProjects,
  terminalUrl,
  type Session,
} from "./session";
import "./demo.css";

export function DemoPage() {
  const [projects, setProjects] = useState<string[]>(sampleProjects);
  const [session, setSession] = useState<Session>();
  const [terminal, setTerminal] = useState("");
  const [draft, setDraft] = useState("");
  const [projectDraft, setProjectDraft] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  const [clearTarget, setClearTarget] = useState<string>();
  const [confirmation, setConfirmation] = useState("");
  const [clearError, setClearError] = useState("");
  const [clearNotice, setClearNotice] = useState("");
  const [graphVersion, setGraphVersion] = useState(0);
  const generation = useRef(0);
  const draftDirty = useRef(false);
  const latestSession = useRef<Session>(undefined);

  function accept(next: Session) {
    latestSession.current = next;
    setSession(next);
    if (!draftDirty.current) setDraft(next.namespace);
    history.replaceState(
      null,
      "",
      `/demo?${new URLSearchParams({ namespace: next.namespace, directory: next.project })}`,
    );
  }
  async function open(project: string, namespace?: string) {
    const current = ++generation.current;
    setBusy(true);
    setError("");
    try {
      const next = await dashboardRequest<Session>("dashboard/sessions", {
        project,
        namespace,
      });
      if (current !== generation.current) return;
      draftDirty.current = false;
      setClearNotice("");
      accept(next);
      setTerminal(terminalUrl(project, next.namespace, next.id));
    } catch (e) {
      setError(String(e));
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    let stopped = false;
    void dashboardRequest<{ projects: string[] }>("projects")
      .then(async ({ projects }) => {
        if (stopped) return;
        setProjects(projects);
        const query = new URLSearchParams(location.search);
        const project = directoryFrom(query.get("directory"));
        await open(
          projects.includes(project) ? project : projects[0],
          query.get("namespace")?.trim() || undefined,
        );
      })
      .catch((e) => {
        if (!stopped) {
          setError(String(e));
          setBusy(false);
        }
      });
    return () => {
      stopped = true;
      generation.current++;
    };
  }, []);
  useEffect(() => {
    if (!session) return;
    let stopped = false;
    const current = generation.current;
    let polling = false;
    const timer = setInterval(async () => {
      if (polling) return;
      polling = true;
      try {
        const next = await dashboardRequest<Session>(
          `dashboard/sessions/${session.id}`,
        );
        if (!stopped && current === generation.current) {
          if (
            latestSession.current?.id === next.id &&
            latestSession.current.revision <= next.revision
          )
            accept(next);
        }
      } catch (e) {
        if (!stopped) setError(String(e));
      } finally {
        polling = false;
      }
    }, 1000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [session?.id]);

  return (
    <div className="pi-demo">
      <header className="demo-header">
        <div className="demo-intro">
          <h1>Pi with shared memory</h1>
          <p>
            Each project has its own memory. Namespace changes sync with the
            live Pi session.
          </p>
        </div>
        <div className="demo-sessions" aria-label="Session directories">
          {projects.map((project) => (
            <Button
              key={project}
              variant={
                session?.project === project ? "primary" : "outline-light"
              }
              aria-pressed={session?.project === project}
              disabled={busy}
              onClick={() => void open(project)}
            >
              Session in {project}
            </Button>
          ))}
        </div>
        <Form
          className="demo-namespace"
          onSubmit={async (event) => {
            event.preventDefault();
            setBusy(true);
            setError("");
            try {
              const { id } = await dashboardRequest<{ id: string }>(
                "projects",
                { id: projectDraft.trim() },
              );
              const result = await dashboardRequest<{ projects: string[] }>(
                "projects",
              );
              setProjects(result.projects);
              setProjectDraft("");
              await open(id);
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <Form.Label htmlFor="new-project">New project</Form.Label>
          <Form.Control
            id="new-project"
            value={projectDraft}
            onChange={(e) => setProjectDraft(e.target.value)}
            pattern="[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}"
            maxLength={64}
            required
            placeholder="my-project"
          />
          <Button type="submit" variant="outline-light" disabled={busy}>
            Create project
          </Button>
        </Form>
        <Form
          className="demo-namespace"
          onSubmit={async (event) => {
            event.preventDefault();
            if (!session) return;
            setBusy(true);
            setError("");
            try {
              const next = await dashboardRequest<Session>(
                `dashboard/sessions/${session.id}`,
                { namespace: draft.trim(), revision: session.revision },
              );
              draftDirty.current = false;
              accept(next);
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <Form.Label htmlFor="demo-namespace">Namespace</Form.Label>
          <Form.Control
            id="demo-namespace"
            value={draft}
            onChange={(e) => {
              draftDirty.current = true;
              setDraft(e.target.value);
            }}
            required
            maxLength={512}
          />
          <Button
            type="submit"
            variant="outline-light"
            disabled={
              busy ||
              !session ||
              !draft.trim() ||
              draft.trim() === session.namespace
            }
          >
            Apply
          </Button>
          <Button
            type="button"
            variant="outline-danger"
            disabled={busy || !session}
            onClick={() => {
              setClearTarget(session!.namespace);
              setConfirmation("");
              setClearError("");
              setClearNotice("");
            }}
          >
            Clear namespace
          </Button>
          <span className="demo-help" role="status">
            {session &&
              (session.revision === session.applied_revision
                ? "Pi namespace synced"
                : "Syncing namespace with Pi…")}
          </span>
        </Form>
        {error && <p role="alert">{error}</p>}
        {clearNotice && <p role="status">{clearNotice}</p>}
      </header>
      <Modal
        show={clearTarget !== undefined}
        onHide={() => {
          if (!busy) setClearTarget(undefined);
        }}
        centered
      >
        <Form
          onSubmit={async (event) => {
            event.preventDefault();
            if (!clearTarget || confirmation !== clearTarget || busy) return;
            if (latestSession.current?.namespace !== clearTarget) {
              setClearError(
                "The active namespace changed. Cancel and open Clear namespace again.",
              );
              return;
            }
            setBusy(true);
            setClearError("");
            try {
              const result = await dashboardRequest<{
                graph?: { cleared?: boolean };
              }>("graph/clear", {
                namespace: clearTarget,
                confirm: confirmation,
              });
              setClearNotice(
                result.graph?.cleared === false
                  ? `Cleared ${clearTarget}. Graph cleanup is queued; the graph will update automatically.`
                  : `Cleared ${clearTarget}.`,
              );
              setGraphVersion((value) => value + 1);
              setClearTarget(undefined);
            } catch (e) {
              setClearError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <Modal.Header closeButton={!busy}>
            <Modal.Title>Clear namespace</Modal.Title>
          </Modal.Header>
          <Modal.Body>
            <p>
              Permanently delete all memories, facts and entities in{" "}
              <strong>{clearTarget}</strong>. This cannot be undone.
            </p>
            <Form.Label htmlFor="clear-confirmation">
              Type the namespace to confirm
            </Form.Label>
            <Form.Control
              id="clear-confirmation"
              autoFocus
              autoComplete="off"
              value={confirmation}
              disabled={busy}
              onChange={(event) => setConfirmation(event.target.value)}
            />
            {clearError && (
              <p role="alert" className="text-danger mt-2">
                {clearError}
              </p>
            )}
          </Modal.Body>
          <Modal.Footer>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setClearTarget(undefined)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              variant="danger"
              disabled={busy || confirmation !== clearTarget}
            >
              {busy ? "Clearing…" : "Delete namespace memory"}
            </Button>
          </Modal.Footer>
        </Form>
      </Modal>
      {session && (
        <main className="demo-panes">
          <section className="demo-pane" aria-labelledby="term-title">
            <div className="demo-pane-heading">
              <h2 id="term-title">Pi, session in {session.project}</h2>
              <Button
                variant="outline-light"
                size="sm"
                disabled={busy}
                onClick={() => void open(session.project, session.namespace)}
              >
                New session
              </Button>
            </div>
            <iframe
              id="term"
              title="Pi terminal"
              allow="clipboard-write"
              src={terminal}
            />
          </section>
          <section className="demo-pane" aria-labelledby="graph-title">
            <div className="demo-pane-heading">
              <h2 id="graph-title">Live memory graph · {session.namespace}</h2>
              <a
                href={graphUrl(session.namespace)}
                target="_blank"
                rel="noopener noreferrer"
              >
                Open graph
              </a>
            </div>
            <iframe
              key={`${session.namespace}:${graphVersion}`}
              id="graph"
              title="Memory graph"
              src={graphUrl(session.namespace)}
            />
          </section>
        </main>
      )}
    </div>
  );
}
