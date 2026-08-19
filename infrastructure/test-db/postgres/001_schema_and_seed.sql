\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE SCHEMA crm;
CREATE SCHEMA billing;
CREATE SCHEMA ops;
CREATE SCHEMA analytics;

CREATE TYPE crm.customer_tier AS ENUM ('free', 'pro', 'enterprise', 'suspended');
CREATE TYPE billing.invoice_status AS ENUM ('draft', 'open', 'paid', 'void', 'uncollectible');
CREATE DOMAIN crm.email_address AS citext
  CHECK (VALUE ~* '^[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}$');

CREATE TABLE crm.accounts (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  external_id text UNIQUE NOT NULL,
  name text NOT NULL CHECK (length(name) >= 2),
  tier crm.customer_tier NOT NULL DEFAULT 'free',
  owner_email crm.email_address NOT NULL,
  tags text[] NOT NULL DEFAULT '{}',
  settings jsonb NOT NULL DEFAULT '{}'::jsonb,
  credit_limit numeric(12, 2) NOT NULL DEFAULT 0 CHECK (credit_limit >= 0),
  trial_period tstzrange,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz,
  search_text text GENERATED ALWAYS AS (lower(name || ' ' || external_id)) STORED
);

CREATE TABLE crm.contacts (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  account_id uuid NOT NULL REFERENCES crm.accounts(id) ON DELETE CASCADE,
  email crm.email_address NOT NULL,
  full_name text NOT NULL,
  title text,
  phone text,
  is_primary boolean NOT NULL DEFAULT false,
  last_seen_at timestamptz,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  UNIQUE (account_id, email)
);

CREATE TABLE billing.plans (
  id text PRIMARY KEY,
  display_name text NOT NULL,
  monthly_price numeric(10, 2) NOT NULL CHECK (monthly_price >= 0),
  included_events integer NOT NULL CHECK (included_events >= 0),
  features jsonb NOT NULL
);

CREATE TABLE billing.subscriptions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id uuid NOT NULL REFERENCES crm.accounts(id) ON DELETE RESTRICT,
  plan_id text NOT NULL REFERENCES billing.plans(id),
  valid_during tstzrange NOT NULL,
  seats integer NOT NULL CHECK (seats > 0),
  cancelled_at timestamptz,
  EXCLUDE USING gist (account_id WITH =, valid_during WITH &&)
);

CREATE TABLE billing.invoices (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id uuid NOT NULL REFERENCES crm.accounts(id),
  subscription_id uuid REFERENCES billing.subscriptions(id),
  invoice_number text NOT NULL UNIQUE,
  status billing.invoice_status NOT NULL,
  subtotal_cents integer NOT NULL CHECK (subtotal_cents >= 0),
  tax_cents integer NOT NULL DEFAULT 0 CHECK (tax_cents >= 0),
  total_cents integer GENERATED ALWAYS AS (subtotal_cents + tax_cents) STORED,
  issued_at timestamptz NOT NULL,
  due_at timestamptz,
  paid_at timestamptz,
  raw_provider_payload jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE billing.invoice_lines (
  invoice_id uuid NOT NULL REFERENCES billing.invoices(id) ON DELETE CASCADE,
  line_no integer NOT NULL,
  sku text NOT NULL,
  description text NOT NULL,
  quantity numeric(10, 2) NOT NULL CHECK (quantity > 0),
  unit_amount_cents integer NOT NULL CHECK (unit_amount_cents >= 0),
  attributes jsonb NOT NULL DEFAULT '{}'::jsonb,
  PRIMARY KEY (invoice_id, line_no)
);

CREATE TABLE ops.deployments (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id uuid REFERENCES crm.accounts(id) ON DELETE SET NULL,
  environment text NOT NULL CHECK (environment IN ('dev', 'staging', 'prod')),
  version text NOT NULL,
  status text NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'rolled_back')),
  requested_by bigint REFERENCES crm.contacts(id) ON DELETE SET NULL,
  started_at timestamptz NOT NULL,
  finished_at timestamptz,
  logs text,
  artifact_sha bytea
);

CREATE TABLE analytics.events (
  id bigserial,
  account_id uuid NOT NULL REFERENCES crm.accounts(id) ON DELETE CASCADE,
  contact_id bigint REFERENCES crm.contacts(id) ON DELETE SET NULL,
  event_name text NOT NULL,
  properties jsonb NOT NULL DEFAULT '{}'::jsonb,
  occurred_at timestamptz NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

CREATE TABLE analytics.events_2026_01 PARTITION OF analytics.events
  FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
CREATE TABLE analytics.events_2026_02 PARTITION OF analytics.events
  FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
CREATE TABLE analytics.events_default PARTITION OF analytics.events DEFAULT;

CREATE INDEX accounts_settings_gin_idx ON crm.accounts USING gin (settings);
CREATE INDEX contacts_account_primary_idx ON crm.contacts (account_id) WHERE is_primary;
CREATE INDEX invoices_account_issued_idx ON billing.invoices (account_id, issued_at DESC);
CREATE INDEX events_account_time_idx ON analytics.events (account_id, occurred_at DESC);
CREATE INDEX events_properties_gin_idx ON analytics.events USING gin (properties);

COMMENT ON TABLE crm.accounts IS 'Seeded customer accounts covering JSON, arrays, enums, generated columns, and ranges.';
COMMENT ON COLUMN ops.deployments.artifact_sha IS 'Binary digest sample to exercise bytea display and export paths.';

INSERT INTO billing.plans (id, display_name, monthly_price, included_events, features) VALUES
  ('free', 'Free', 0, 1000, '{"support":"community","retention_days":7}'::jsonb),
  ('pro', 'Pro', 49, 250000, '{"support":"email","retention_days":90,"exports":["csv","json"]}'::jsonb),
  ('enterprise', 'Enterprise', 999, 10000000, '{"support":"dedicated","sso":true,"retention_days":730}'::jsonb);

INSERT INTO crm.accounts (id, external_id, name, tier, owner_email, tags, settings, credit_limit, trial_period, created_at, updated_at) VALUES
  ('00000000-0000-4000-8000-000000000001', 'acct_acme', 'Acme Analytics', 'enterprise', 'ops@acme.example', ARRAY['priority','sso'], '{"timezone":"America/New_York","feature_flags":{"beta_grid":true},"limits":{"users":500}}', 50000, tstzrange('2026-01-01', '2026-02-01', '[)'), '2026-01-02 10:00+00', '2026-02-03 12:30+00'),
  ('00000000-0000-4000-8000-000000000002', 'acct_zen', 'Zenith Retail', 'pro', 'admin@zenith.example', ARRAY['retail','overdue'], '{"timezone":"Asia/Kolkata","feature_flags":{"beta_grid":false},"notes":"contains unicode cafe"}', 2500, tstzrange('2026-02-01', NULL, '[)'), '2026-02-08 08:15+00', NULL),
  ('00000000-0000-4000-8000-000000000003', 'acct_nulls', 'Null Island Labs', 'free', 'hello@null-island.example', ARRAY[]::text[], '{"timezone":null,"feature_flags":{},"empty_object":{}}', 0, NULL, '2026-02-20 16:45+00', NULL),
  ('00000000-0000-4000-8000-000000000004', 'acct_suspended', 'Suspended Co', 'suspended', 'billing@suspended.example', ARRAY['risk','readonly'], '{"locked_reason":"payment_failed","large_text":"This row intentionally has awkward edge-case values."}', 100, NULL, '2026-01-20 09:00+00', '2026-02-21 09:00+00');

INSERT INTO crm.contacts (id, account_id, email, full_name, title, phone, is_primary, last_seen_at, metadata) OVERRIDING SYSTEM VALUE VALUES
  (101, '00000000-0000-4000-8000-000000000001', 'ada@acme.example', 'Ada Lovelace', 'CTO', '+1-555-0101', true, '2026-02-25 14:00+00', '{"preferred_channel":"email"}'),
  (102, '00000000-0000-4000-8000-000000000001', 'grace@acme.example', 'Grace Hopper', NULL, NULL, false, NULL, '{"notes":["missing phone","missing title"]}'),
  (201, '00000000-0000-4000-8000-000000000002', 'radha@zenith.example', 'Radha Raman', 'Operations Lead', '+91-80-555-0199', true, '2026-02-27 05:45+00', '{"locale":"en-IN"}'),
  (301, '00000000-0000-4000-8000-000000000003', 'nobody@null-island.example', 'Nobody Null', 'Researcher', NULL, true, NULL, '{}');

INSERT INTO billing.subscriptions (id, account_id, plan_id, valid_during, seats, cancelled_at) VALUES
  ('10000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000001', 'enterprise', tstzrange('2026-01-01', NULL, '[)'), 180, NULL),
  ('10000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000002', 'pro', tstzrange('2026-02-01', NULL, '[)'), 24, NULL),
  ('10000000-0000-4000-8000-000000000003', '00000000-0000-4000-8000-000000000004', 'pro', tstzrange('2026-01-01', '2026-02-20', '[)'), 5, '2026-02-20 09:00+00');

INSERT INTO billing.invoices (id, account_id, subscription_id, invoice_number, status, subtotal_cents, tax_cents, issued_at, due_at, paid_at, raw_provider_payload) VALUES
  ('20000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000001', '10000000-0000-4000-8000-000000000001', 'INV-2026-0001', 'paid', 99900, 7992, '2026-02-01 00:00+00', '2026-02-15 00:00+00', '2026-02-02 10:15+00', '{"provider":"stripe","attempts":1}'),
  ('20000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000002', '10000000-0000-4000-8000-000000000002', 'INV-2026-0002', 'open', 4900, 882, '2026-02-10 00:00+00', '2026-02-24 00:00+00', NULL, '{"provider":"manual","warning":"past_due_soon"}'),
  ('20000000-0000-4000-8000-000000000003', '00000000-0000-4000-8000-000000000004', '10000000-0000-4000-8000-000000000003', 'INV-2026-0003', 'uncollectible', 4900, 0, '2026-02-01 00:00+00', '2026-02-15 00:00+00', NULL, '{"provider":"stripe","error":{"code":"card_declined"}}');

INSERT INTO billing.invoice_lines (invoice_id, line_no, sku, description, quantity, unit_amount_cents, attributes) VALUES
  ('20000000-0000-4000-8000-000000000001', 1, 'enterprise-base', 'Enterprise base subscription', 1, 99900, '{"period":"monthly"}'),
  ('20000000-0000-4000-8000-000000000001', 2, 'discount', 'Contract discount represented as zero-cost line', 1, 0, '{"discount":true}'),
  ('20000000-0000-4000-8000-000000000002', 1, 'pro-base', 'Pro subscription', 1, 4900, '{"period":"monthly"}'),
  ('20000000-0000-4000-8000-000000000003', 1, 'pro-base', 'Pro subscription before suspension', 1, 4900, '{}');

INSERT INTO ops.deployments (id, account_id, environment, version, status, requested_by, started_at, finished_at, logs, artifact_sha) VALUES
  ('30000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000001', 'prod', '2026.02.25.1', 'succeeded', 101, '2026-02-25 10:00+00', '2026-02-25 10:08+00', 'migrated 3 tables' || chr(10) || 'backfill complete', decode('DEADBEEF', 'hex')),
  ('30000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000002', 'staging', '2026.02.27.3', 'failed', 201, '2026-02-27 11:30+00', '2026-02-27 11:32+00', repeat('timeout while polling healthcheck ', 8), decode('00FFAA10', 'hex')),
  ('30000000-0000-4000-8000-000000000003', NULL, 'dev', 'scratch', 'running', NULL, '2026-02-28 05:00+00', NULL, NULL, NULL);

INSERT INTO analytics.events (account_id, contact_id, event_name, properties, occurred_at, received_at) VALUES
  ('00000000-0000-4000-8000-000000000001', 101, 'workspace.opened', '{"route":"/tables","duration_ms":151}', '2026-01-11 09:00+00', '2026-01-11 09:00:01+00'),
  ('00000000-0000-4000-8000-000000000001', 102, 'query.failed', '{"sqlstate":"42601","message":"syntax error at or near SELECT"}', '2026-02-18 17:00+00', '2026-02-18 17:00:02+00'),
  ('00000000-0000-4000-8000-000000000002', 201, 'export.started', '{"format":"csv","rows":125000}', '2026-02-21 07:45+00', '2026-02-21 07:45:01+00'),
  ('00000000-0000-4000-8000-000000000003', 301, 'empty_state.viewed', '{"component":"schema-map","relationships":0}', '2026-03-01 12:00+00', '2026-03-01 12:00:01+00'),
  ('00000000-0000-4000-8000-000000000004', NULL, 'connection.rejected', '{"reason":"suspended_account","retryable":false}', '2026-02-20 09:05+00', '2026-02-20 09:05:01+00');

CREATE VIEW crm.account_overview AS
SELECT
  a.id,
  a.external_id,
  a.name,
  a.tier,
  a.owner_email,
  count(DISTINCT c.id) AS contact_count,
  count(DISTINCT i.id) AS invoice_count,
  coalesce(sum(i.total_cents), 0) AS lifetime_cents
FROM crm.accounts a
LEFT JOIN crm.contacts c ON c.account_id = a.id
LEFT JOIN billing.invoices i ON i.account_id = a.id
GROUP BY a.id;

CREATE MATERIALIZED VIEW analytics.event_counts_by_account AS
SELECT
  account_id,
  event_name,
  count(*) AS event_count,
  min(occurred_at) AS first_seen_at,
  max(occurred_at) AS last_seen_at
FROM analytics.events
GROUP BY account_id, event_name;

CREATE SCHEMA browse_fixture;

CREATE TABLE browse_fixture.keyless_dupes (
  label text NOT NULL,
  amount integer NOT NULL
);
INSERT INTO browse_fixture.keyless_dupes (label, amount)
SELECT 'dup-' || (g % 10), g % 5
FROM generate_series(1, 40) AS g;

CREATE TABLE browse_fixture.expr_unique (
  id integer,
  email text
);
CREATE UNIQUE INDEX expr_unique_lower_email
  ON browse_fixture.expr_unique (lower(email));
INSERT INTO browse_fixture.expr_unique (id, email) VALUES
  (1, 'Ada@example.com'),
  (1, 'ada@other.example'),
  (NULL, 'nobody@example.com');

CREATE TABLE browse_fixture.keyless_parts (
  label text NOT NULL,
  amount integer NOT NULL
) PARTITION BY LIST (label);
CREATE TABLE browse_fixture.keyless_parts_a
  PARTITION OF browse_fixture.keyless_parts FOR VALUES IN ('a');
CREATE TABLE browse_fixture.keyless_parts_b
  PARTITION OF browse_fixture.keyless_parts FOR VALUES IN ('b');
INSERT INTO browse_fixture.keyless_parts (label, amount)
SELECT CASE WHEN g % 2 = 0 THEN 'a' ELSE 'b' END, g
FROM generate_series(1, 20) AS g;

CREATE TABLE browse_fixture.large_rows (
  id bigint PRIMARY KEY,
  payload text NOT NULL
);
INSERT INTO browse_fixture.large_rows (id, payload)
SELECT g, 'row-' || g
FROM generate_series(1, 100000) AS g;
