import { useState } from "react";
import Alert from "react-bootstrap/Alert";
import Button from "react-bootstrap/Button";
import Form from "react-bootstrap/Form";
import { api } from "../api";
import { errorMessage } from "../hooks/useResource";
import type { RecallResponse } from "../types";
import { ErrorNotice, Panel, StatusBadge } from "./Panel";

export function RecallPanel({
  namespace,
  onRecalled,
}: {
  namespace: string;
  onRecalled: (traceId: string) => void;
}) {
  const [query, setQuery] = useState(
    "What language does Aryan like to code in?",
  );
  const [tokens, setTokens] = useState(2048);
  const [graph, setGraph] = useState(true);
  const [observations, setObservations] = useState(true);
  const [result, setResult] = useState<RecallResponse | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function recall() {
    setBusy(true);
    setError("");
    setResult(null);
    try {
      const response = await api.recall(
        namespace,
        query,
        tokens,
        graph,
        observations,
      );
      setResult(response);
      onRecalled(response.trace_id);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel title="Recall memory">
      <p className="text-secondary small">
        Ask a question to retrieve related evidence within a token budget.
      </p>
      <Form
        onSubmit={(event) => {
          event.preventDefault();
          void recall();
        }}
      >
        <fieldset disabled={busy || !namespace.trim()}>
          <Form.Group className="mb-3" controlId="question">
            <Form.Label>Question</Form.Label>
            <Form.Control
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              required
            />
          </Form.Group>
          <Form.Group className="mb-3" controlId="tokens">
            <Form.Label>Maximum context tokens</Form.Label>
            <Form.Control
              type="number"
              min={256}
              max={32768}
              value={tokens}
              onChange={(event) => setTokens(Number(event.target.value))}
              required
            />
          </Form.Group>
          <div className="d-flex flex-wrap gap-3 mb-3">
            <Form.Check
              id="graph-expansion"
              label="Graph expansion"
              checked={graph}
              onChange={(event) => setGraph(event.target.checked)}
            />
            <Form.Check
              id="observations"
              label="Consolidated observations"
              checked={observations}
              onChange={(event) => setObservations(event.target.checked)}
            />
          </div>
          <Button type="submit" disabled={!query.trim()}>
            {busy ? "Searching…" : "Recall evidence"}
          </Button>
        </fieldset>
      </Form>
      <div className="mt-3" aria-live="polite">
        <ErrorNotice message={error} />
        {result ? (
          <>
            {result.degraded_reasons.length ? (
              <Alert variant="warning">
                {result.degraded_reasons.join(" · ")}
              </Alert>
            ) : null}
            <pre className="data-output">
              {result.context || "No matching evidence found."}
            </pre>
            <p className="small text-secondary">
              {result.estimated_tokens} estimated tokens · {result.elapsed_ms}{" "}
              ms
            </p>
            <details>
              <summary className="small">
                Ranked matches ({result.ranking.length})
              </summary>
              <ol className="mt-2 ps-3">
                {result.ranking.map((hit) => (
                  <li key={hit.id} className="mb-2">
                    {hit.statement} <StatusBadge status={hit.status} />
                    <div className="small text-secondary">
                      {Object.entries(hit.ranks)
                        .map(([channel, rank]) => `${channel}: ${rank}`)
                        .join(" · ")}
                    </div>
                  </li>
                ))}
              </ol>
            </details>
          </>
        ) : null}
      </div>
    </Panel>
  );
}
