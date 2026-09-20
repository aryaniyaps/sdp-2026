-- A retried extraction must not replay the legacy resolver out of order.
CREATE TABLE legacy_resolution_receipts (
 namespace text NOT NULL,
 receipt_key text NOT NULL,
 outcome jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(namespace, receipt_key)
);
