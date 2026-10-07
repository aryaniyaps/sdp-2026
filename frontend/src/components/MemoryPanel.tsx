import { useCallback, useState } from "react";
import Button from "react-bootstrap/Button";
import Modal from "react-bootstrap/Modal";
import Table from "react-bootstrap/Table";
import { api } from "../api";
import { useResource } from "../hooks/useResource";
import type { GraphSnapshot } from "../types";
import { ErrorNotice, JsonDetails, Panel, StatusBadge } from "./Panel";

export function MemoryPanel({
  namespace,
  snapshot,
  loading,
  error,
  onRefresh,
}: {
  namespace: string;
  snapshot: GraphSnapshot | null;
  loading: boolean;
  error: string;
  onRefresh: () => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const loadDetail = useCallback(
    (signal: AbortSignal) =>
      selected
        ? api.assertion(namespace, selected, signal)
        : Promise.resolve(null),
    [namespace, selected],
  );
  const detail = useResource(loadDetail);

  return (
    <Panel
      title="Memory history"
      action={
        <Button
          size="sm"
          variant="outline-secondary"
          disabled={loading}
          onClick={onRefresh}
        >
          Refresh
        </Button>
      }
    >
      <ErrorNotice message={error} />
      {loading ? (
        <p className="text-secondary" role="status">
          Loading memory…
        </p>
      ) : null}
      {snapshot && !snapshot.assertions.length ? (
        <p className="text-secondary mb-0">
          No facts yet. Save an interaction, then refresh once extraction
          finishes.
        </p>
      ) : null}
      {snapshot?.assertions.length ? (
        <Table responsive hover className="mb-0 align-middle">
          <thead>
            <tr>
              <th>Statement</th>
              <th>Status</th>
              <th>Valid from</th>
              <th>
                <span className="visually-hidden">Details</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {snapshot.assertions.map((assertion) => (
              <tr key={assertion.id}>
                <td>
                  {assertion.statement}
                  <div className="small text-secondary">
                    {assertion.kind} · confidence {assertion.confidence}
                  </div>
                </td>
                <td>
                  <StatusBadge status={assertion.status} />
                </td>
                <td className="small">
                  {assertion.valid_from
                    ? new Date(assertion.valid_from).toLocaleDateString()
                    : "Unknown"}
                  {assertion.valid_to
                    ? ` – ${new Date(assertion.valid_to).toLocaleDateString()}`
                    : ""}
                </td>
                <td>
                  <Button
                    size="sm"
                    variant="outline-secondary"
                    onClick={() => setSelected(assertion.id)}
                    aria-label={`Inspect evidence for ${assertion.statement}`}
                  >
                    Evidence
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
      ) : null}
      <Modal
        show={selected !== null}
        onHide={() => setSelected(null)}
        size="lg"
      >
        <Modal.Header closeButton>
          <Modal.Title className="h5">Evidence and dependencies</Modal.Title>
        </Modal.Header>
        <Modal.Body>
          <ErrorNotice message={detail.error} />
          {detail.loading ? (
            <p role="status">Loading evidence…</p>
          ) : (
            <JsonDetails value={detail.data} />
          )}
        </Modal.Body>
      </Modal>
    </Panel>
  );
}
