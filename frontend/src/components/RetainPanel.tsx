import { useState } from "react";
import Alert from "react-bootstrap/Alert";
import Button from "react-bootstrap/Button";
import Form from "react-bootstrap/Form";
import { api } from "../api";
import { errorMessage } from "../hooks/useResource";
import { ErrorNotice, Panel } from "./Panel";

export function RetainPanel({
  namespace,
  onSaved,
}: {
  namespace: string;
  onSaved: () => void;
}) {
  const [session, setSession] = useState("review-session");
  const [text, setText] = useState("Aryan prefers Python for programming.");
  const [result, setResult] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    setError("");
    setResult("");
    try {
      const saved = await api.retain(namespace, session, text);
      setResult(
        `Evidence saved. Job ${saved.job_id} will extract facts in the background.`,
      );
      onSaved();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel title="Save an interaction">
      <p className="text-secondary small">
        Keep the original text as evidence. Facts appear after extraction
        finishes.
      </p>
      <Form
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <fieldset disabled={busy || !namespace.trim()}>
          <Form.Group className="mb-3" controlId="session">
            <Form.Label>Session</Form.Label>
            <Form.Control
              value={session}
              onChange={(event) => setSession(event.target.value)}
              required
            />
          </Form.Group>
          <Form.Group className="mb-3" controlId="interaction">
            <Form.Label>Interaction text</Form.Label>
            <Form.Control
              as="textarea"
              rows={4}
              value={text}
              onChange={(event) => setText(event.target.value)}
              required
            />
          </Form.Group>
          <div className="d-flex flex-wrap gap-2">
            <Button type="submit" disabled={!text.trim() || !session.trim()}>
              {busy ? "Saving…" : "Retain evidence"}
            </Button>
            <Button
              variant="outline-secondary"
              onClick={() => setText("Aryan now prefers Rust for programming.")}
            >
              Try a correction
            </Button>
          </div>
        </fieldset>
      </Form>
      <div className="mt-3" aria-live="polite">
        <ErrorNotice message={error} />
        {result ? (
          <Alert variant="success" className="mb-0 result-message">
            {result}
          </Alert>
        ) : null}
      </div>
    </Panel>
  );
}
