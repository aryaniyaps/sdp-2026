import { useState } from "react";
import Button from "react-bootstrap/Button";
import Col from "react-bootstrap/Col";
import ListGroup from "react-bootstrap/ListGroup";
import Row from "react-bootstrap/Row";
import type { Trace } from "../types";
import { ErrorNotice, JsonDetails, Panel, StatusBadge } from "./Panel";

export function TracePanel({
  traces,
  preferredId,
  loading,
  error,
  onRefresh,
}: {
  traces: Trace[] | null;
  preferredId: string | null;
  loading: boolean;
  error: string;
  onRefresh: () => void;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected =
    traces?.find((trace) => trace.id === (selectedId ?? preferredId)) ??
    traces?.[0];

  return (
    <Panel
      title="Operation traces"
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
        <p role="status" className="text-secondary">
          Loading operations…
        </p>
      ) : null}
      {!loading && !traces?.length ? (
        <p className="text-secondary mb-0">
          Save or recall an interaction to see its processing steps.
        </p>
      ) : null}
      {traces?.length ? (
        <Row className="g-3">
          <Col md={4}>
            <ListGroup className="trace-list">
              {traces.map((trace) => (
                <ListGroup.Item
                  action
                  key={trace.id}
                  active={trace.id === selected?.id}
                  onClick={() => setSelectedId(trace.id)}
                >
                  <div className="d-flex justify-content-between gap-2">
                    <span>{trace.operation_type}</span>
                    <StatusBadge status={trace.status} />
                  </div>
                  <div className="small mt-1">
                    {trace.duration_ms} ms ·{" "}
                    {new Date(trace.started_at).toLocaleTimeString()}
                  </div>
                </ListGroup.Item>
              ))}
            </ListGroup>
          </Col>
          <Col md={8}>
            {selected ? (
              <>
                <p className="small text-secondary text-break">
                  Operation {selected.id}
                </p>
                <details className="mb-3">
                  <summary>Request</summary>
                  <JsonDetails value={selected.request} />
                </details>
                {selected.steps.map((step) => (
                  <div key={step.ordinal} className="trace-step">
                    <div className="d-flex flex-wrap align-items-center gap-2">
                      <strong>
                        {step.ordinal + 1}. {step.stage}
                      </strong>
                      <StatusBadge status={step.status} />
                      <span className="small text-secondary">
                        {step.duration_ms} ms
                      </span>
                    </div>
                    <JsonDetails value={step.details} />
                  </div>
                ))}
              </>
            ) : null}
          </Col>
        </Row>
      ) : null}
    </Panel>
  );
}
