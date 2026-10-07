import { useCallback, useState } from "react";
import Badge from "react-bootstrap/Badge";
import Button from "react-bootstrap/Button";
import Col from "react-bootstrap/Col";
import Container from "react-bootstrap/Container";
import Form from "react-bootstrap/Form";
import Navbar from "react-bootstrap/Navbar";
import Row from "react-bootstrap/Row";
import { api } from "./api";
import { useResource } from "./hooks/useResource";
import { RetainPanel } from "./components/RetainPanel";
import { RecallPanel } from "./components/RecallPanel";
import { MemoryPanel } from "./components/MemoryPanel";
import { TracePanel } from "./components/TracePanel";
import { ProcessingPanel } from "./components/ProcessingPanel";
import { ErrorNotice } from "./components/Panel";

export default function App() {
  const [namespace, setNamespace] = useState(
    () => new URLSearchParams(location.search).get("namespace") || "review",
  );
  const [draftNamespace, setDraftNamespace] = useState(namespace);
  const [traceId, setTraceId] = useState<string | null>(null);
  const health = useResource(
    useCallback((signal: AbortSignal) => api.health(signal), []),
  );
  const graph = useResource(
    useCallback(
      (signal: AbortSignal) => api.graph(namespace, signal),
      [namespace],
    ),
  );
  const traces = useResource(
    useCallback(
      (signal: AbortSignal) => api.traces(namespace, signal),
      [namespace],
    ),
  );

  function refresh() {
    graph.refresh();
    traces.refresh();
    health.refresh();
  }
  function changeNamespace() {
    const next = draftNamespace.trim();
    if (!next) return;
    setNamespace(next);
    setTraceId(null);
    history.replaceState(
      null,
      "",
      `/?${new URLSearchParams({ namespace: next })}`,
    );
  }

  return (
    <>
      <Navbar className="border-bottom bg-white">
        <Container className="gap-3 flex-wrap">
          <Navbar.Brand className="fw-semibold">Memory Engine</Navbar.Brand>
          <div className="d-flex align-items-center gap-3">
            <Badge bg={health.data?.database ? "success" : "secondary"}>
              {health.loading
                ? "Checking service…"
                : health.data?.database
                  ? "Database connected"
                  : "Service unavailable"}
            </Badge>
            <a href="/swagger-ui/" className="small">
              API docs
            </a>
          </div>
        </Container>
      </Navbar>
      <Container as="main" className="py-4">
        <div className="mb-4">
          <h1 className="h3">Memory workspace</h1>
          <p className="text-secondary mb-3">
            Save interactions, recall evidence, and inspect how facts change
            over time.
          </p>
          <Form
            onSubmit={(event) => {
              event.preventDefault();
              changeNamespace();
            }}
            className="namespace-form d-flex align-items-end gap-2"
          >
            <Form.Group controlId="namespace" className="flex-grow-1">
              <Form.Label className="small">Namespace</Form.Label>
              <Form.Control
                value={draftNamespace}
                onChange={(event) => setDraftNamespace(event.target.value)}
                required
              />
            </Form.Group>
            <Button
              type="submit"
              variant="outline-primary"
              disabled={!draftNamespace.trim()}
            >
              Load
            </Button>
          </Form>
          <div className="small text-secondary mt-2">
            Showing <strong>{namespace}</strong> · embedding{" "}
            {health.data?.embedder ? "ready" : "unavailable"} · worker{" "}
            {health.data?.worker_model || "unknown"}
          </div>
        </div>
        <ErrorNotice message={health.error} />
        <Row className="g-4">
          <Col lg={6}>
            <RetainPanel
              key={`retain-${namespace}`}
              namespace={namespace}
              onSaved={refresh}
            />
          </Col>
          <Col lg={6}>
            <RecallPanel
              key={`recall-${namespace}`}
              namespace={namespace}
              onRecalled={(id) => {
                setTraceId(id);
                refresh();
              }}
            />
          </Col>
          <Col xs={12}>
            <ProcessingPanel namespace={namespace} snapshot={graph.data} />
          </Col>
          <Col xs={12}>
            <MemoryPanel
              key={`memory-${namespace}`}
              namespace={namespace}
              snapshot={graph.data}
              loading={graph.loading}
              error={graph.error}
              onRefresh={graph.refresh}
            />
          </Col>
          <Col xs={12}>
            <TracePanel
              key={`${namespace}-${traceId}`}
              traces={traces.data}
              preferredId={traceId}
              loading={traces.loading}
              error={traces.error}
              onRefresh={traces.refresh}
            />
          </Col>
        </Row>
        <footer className="small text-secondary mt-4">
          Evidence is saved immediately. Extracted facts and the graph update in
          the background.
        </footer>
      </Container>
    </>
  );
}
