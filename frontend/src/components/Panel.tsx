import type { ReactNode } from "react";
import Card from "react-bootstrap/Card";
import Badge from "react-bootstrap/Badge";
import Alert from "react-bootstrap/Alert";

export function Panel({
  title,
  action,
  children,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Card className="h-100">
      <Card.Header className="d-flex justify-content-between align-items-center gap-3">
        <h2 className="h6 mb-0">{title}</h2>
        {action}
      </Card.Header>
      <Card.Body>{children}</Card.Body>
    </Card>
  );
}

export function StatusBadge({ status }: { status: string }) {
  const variant = ["active", "succeeded", "ok"].includes(status)
    ? "success"
    : ["failed", "error", "retracted"].includes(status)
      ? "danger"
      : "secondary";
  return <Badge bg={variant}>{status}</Badge>;
}

export function ErrorNotice({ message }: { message: string }) {
  return message ? (
    <Alert variant="danger" className="mb-3">
      {message}
    </Alert>
  ) : null;
}

export function JsonDetails({ value }: { value: unknown }) {
  return (
    <pre className="data-output mb-0">{JSON.stringify(value, null, 2)}</pre>
  );
}
