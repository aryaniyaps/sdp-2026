import Button from "react-bootstrap/Button";
import Table from "react-bootstrap/Table";
import type { GraphSnapshot } from "../types";
import { Panel, StatusBadge } from "./Panel";

export function ProcessingPanel({
  namespace,
  snapshot,
}: {
  namespace: string;
  snapshot: GraphSnapshot | null;
}) {
  return (
    <Panel
      title="Graph and processing"
      action={
        <Button
          size="sm"
          variant="outline-primary"
          href={`/graph?${new URLSearchParams({ namespace })}`}
          disabled={!namespace.trim()}
        >
          Open graph
        </Button>
      }
    >
      {snapshot ? (
        <>
          <div className="d-flex flex-wrap gap-4 mb-3">
            <div>
              <strong>{snapshot.entities.length}</strong>
              <span className="text-secondary ms-2">entities shown</span>
            </div>
            <div>
              <strong>{snapshot.assertions.length}</strong>
              <span className="text-secondary ms-2">facts shown</span>
            </div>
            <div>
              <strong>{snapshot.status.outstanding_jobs}</strong>
              <span className="text-secondary ms-2">outstanding jobs</span>
            </div>
            <div>
              <strong>{snapshot.status.stale_observations}</strong>
              <span className="text-secondary ms-2">stale observations</span>
            </div>
          </div>
          <p className="small text-secondary">
            {snapshot.status.ready
              ? "Processing is complete."
              : "There is pending or failed work."}{" "}
            Last projection:{" "}
            {snapshot.status.last_projection_completed_at
              ? new Date(
                  snapshot.status.last_projection_completed_at,
                ).toLocaleString()
              : "Not completed yet"}
            .
          </p>
          {snapshot.status.jobs.length ? (
            <Table size="sm" className="mb-0">
              <thead>
                <tr>
                  <th>Job</th>
                  <th>Status</th>
                  <th>Count</th>
                </tr>
              </thead>
              <tbody>
                {snapshot.status.jobs.map((job) => (
                  <tr key={`${job.kind}-${job.status}`}>
                    <td>{job.kind}</td>
                    <td>
                      <StatusBadge status={job.status} />
                    </td>
                    <td>{job.count}</td>
                  </tr>
                ))}
              </tbody>
            </Table>
          ) : null}
        </>
      ) : (
        <p className="text-secondary mb-0">
          Select a namespace to inspect its graph and queue.
        </p>
      )}
    </Panel>
  );
}
