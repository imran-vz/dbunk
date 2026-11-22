-- Demo data for UI council screenshots. Loaded only into the owned, disposable
-- native fixture (tools/native/fixture.py) by capture.py; idempotent.
DROP SCHEMA IF EXISTS billing CASCADE;
CREATE SCHEMA billing;
SET search_path = billing;

CREATE TYPE invoice_status AS ENUM ('draft', 'open', 'paid', 'void', 'uncollectible');

CREATE TABLE plans (
  id          smallint PRIMARY KEY,
  code        text NOT NULL UNIQUE,
  name        text NOT NULL,
  monthly_usd numeric(10,2) NOT NULL,
  seats       integer,
  is_public   boolean NOT NULL DEFAULT true
);

CREATE TABLE customers (
  id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  email       text NOT NULL UNIQUE,
  full_name   text NOT NULL,
  company     text,
  country     char(2) NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now(),
  metadata    jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE subscriptions (
  id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  customer_id bigint NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
  plan_id     smallint NOT NULL REFERENCES plans(id),
  started_at  timestamptz NOT NULL,
  cancelled_at timestamptz,
  trial       boolean NOT NULL DEFAULT false
);

CREATE TABLE invoices (
  id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  number      text NOT NULL UNIQUE,
  customer_id bigint NOT NULL REFERENCES customers(id),
  subscription_id bigint REFERENCES subscriptions(id),
  status      invoice_status NOT NULL DEFAULT 'draft',
  amount_usd  numeric(12,2) NOT NULL,
  tax_usd     numeric(12,2) NOT NULL DEFAULT 0,
  issued_at   timestamptz NOT NULL,
  due_at      date NOT NULL,
  paid_at     timestamptz,
  notes       text
);
CREATE INDEX invoices_customer_idx ON invoices (customer_id);
CREATE INDEX invoices_status_issued_idx ON invoices (status, issued_at DESC);

CREATE TABLE payments (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  invoice_id  bigint NOT NULL REFERENCES invoices(id),
  method      text NOT NULL CHECK (method IN ('card', 'ach', 'wire', 'credit')),
  amount_usd  numeric(12,2) NOT NULL,
  received_at timestamptz NOT NULL,
  refunded    boolean NOT NULL DEFAULT false
);

INSERT INTO plans VALUES
  (1, 'free', 'Free', 0, 1, true),
  (2, 'starter', 'Starter', 19, 3, true),
  (3, 'team', 'Team', 79, 15, true),
  (4, 'business', 'Business', 249, 50, true),
  (5, 'enterprise', 'Enterprise', 1200, NULL, false);

INSERT INTO customers (email, full_name, company, country, created_at, metadata)
SELECT
  lower(f) || '.' || lower(l) || g || '@' || d,
  f || ' ' || l,
  CASE WHEN g % 4 = 0 THEN NULL ELSE c END,
  (ARRAY['US','GB','DE','IN','FR','JP','CA','BR','NL','SE'])[1 + g % 10],
  timestamptz '2025-01-03 09:00+00' + (g * interval '7 hours 13 minutes'),
  jsonb_build_object('source', (ARRAY['organic','referral','ads','partner'])[1 + g % 4], 'seats', 1 + g % 17)
FROM generate_series(1, 480) AS g,
LATERAL (SELECT
  (ARRAY['Ada','Grace','Linus','Margaret','Ken','Barbara','Dennis','Radia','Guido','Frances','Edsger','Hedy','Alan','Joan','Tim','Sophie'])[1 + g % 16] AS f,
  (ARRAY['Lovelace','Hopper','Torvalds','Hamilton','Thompson','Liskov','Ritchie','Perlman','Rossum','Allen','Dijkstra','Lamarr','Kay','Clarke','Berners','Wilson','Okafor','Tanaka','Silva','Novak'])[1 + (g * 7) % 20] AS l,
  (ARRAY['Northwind','Initech','Globex','Umbrella Labs','Hooli','Stark Analytics','Acme Cloud','Vandelay','Wayne Data','Soylent'])[1 + (g * 3) % 10] AS c,
  (ARRAY['example.com','example.org','example.net'])[1 + g % 3] AS d) AS n;

INSERT INTO subscriptions (customer_id, plan_id, started_at, cancelled_at, trial)
SELECT id, 1 + (id % 5)::smallint, created_at + interval '2 days',
  CASE WHEN id % 9 = 0 THEN created_at + interval '120 days' END, id % 11 = 0
FROM customers;

INSERT INTO invoices (number, customer_id, subscription_id, status, amount_usd, tax_usd, issued_at, due_at, paid_at, notes)
SELECT
  'INV-' || to_char(2025, 'FM0000') || '-' || lpad(g::text, 5, '0'),
  s.customer_id, s.id,
  (ARRAY['paid','paid','paid','open','draft','void','uncollectible','paid'])[1 + g % 8]::invoice_status,
  round((p.monthly_usd * (1 + (g % 3)))::numeric, 2),
  round((p.monthly_usd * (1 + (g % 3)) * 0.08)::numeric, 2),
  s.started_at + (g % 12) * interval '30 days',
  (s.started_at + (g % 12) * interval '30 days' + interval '14 days')::date,
  CASE WHEN g % 8 IN (0, 1, 2, 7) THEN s.started_at + (g % 12) * interval '30 days' + interval '3 days' END,
  CASE WHEN g % 13 = 0 THEN 'Customer requested PO number on invoice' END
FROM generate_series(1, 2400) AS g
JOIN subscriptions s ON s.id = 1 + (g % 480)
JOIN plans p ON p.id = s.plan_id;

INSERT INTO payments (invoice_id, method, amount_usd, received_at, refunded)
SELECT id, (ARRAY['card','card','ach','wire','credit'])[1 + id % 5], amount_usd + tax_usd, paid_at, id % 37 = 0
FROM invoices WHERE paid_at IS NOT NULL;

CREATE VIEW monthly_revenue AS
SELECT date_trunc('month', paid_at)::date AS month, count(*) AS invoices, sum(amount_usd) AS revenue_usd
FROM invoices WHERE status = 'paid' GROUP BY 1 ORDER BY 1;

CREATE FUNCTION customer_lifetime_value(customer bigint) RETURNS numeric
LANGUAGE sql STABLE AS $$
  SELECT coalesce(sum(amount_usd), 0) FROM billing.invoices WHERE customer_id = customer AND status = 'paid'
$$;

COMMENT ON TABLE invoices IS 'One row per issued invoice; amounts exclude tax.';
ANALYZE;
