CREATE DATABASE IF NOT EXISTS dbunk_demo;

CREATE TABLE IF NOT EXISTS dbunk_demo.accounts
(
  account_id UUID,
  external_id String,
  name String,
  tier Enum8('free' = 1, 'pro' = 2, 'enterprise' = 3, 'suspended' = 4),
  region LowCardinality(String),
  tags Array(String),
  settings Map(String, String),
  credit_limit Decimal(12, 2),
  created_at DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY (tier, account_id);

CREATE TABLE IF NOT EXISTS dbunk_demo.sessions
(
  session_id UUID,
  account_id UUID,
  user_id UInt64,
  started_at DateTime64(3, 'UTC'),
  ended_at Nullable(DateTime64(3, 'UTC')),
  user_agent String,
  ip_address IPv4,
  country FixedString(2),
  is_bot Bool
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(started_at)
ORDER BY (account_id, started_at, session_id);

CREATE TABLE IF NOT EXISTS dbunk_demo.events
(
  event_id UUID,
  account_id UUID,
  session_id UUID,
  event_name LowCardinality(String),
  event_time DateTime64(3, 'UTC'),
  ingest_time DateTime64(3, 'UTC') DEFAULT now64(3),
  revenue Decimal(12, 2),
  duration_ms Nullable(UInt32),
  attributes Map(String, String),
  raw_json String,
  version UInt32
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(event_time)
ORDER BY (account_id, event_name, event_time, event_id);

CREATE TABLE IF NOT EXISTS dbunk_demo.event_items
(
  event_id UUID,
  sku String,
  quantity UInt32,
  price Decimal(12, 2),
  discounts Array(Decimal(12, 2))
)
ENGINE = MergeTree
ORDER BY (event_id, sku);

CREATE TABLE IF NOT EXISTS dbunk_demo.metric_rollups
(
  rollup_date Date,
  account_id UUID,
  event_name LowCardinality(String),
  events UInt64,
  revenue Decimal(18, 2),
  max_duration_ms UInt32
)
ENGINE = SummingMergeTree((events, revenue))
PARTITION BY toYYYYMM(rollup_date)
ORDER BY (rollup_date, account_id, event_name);

CREATE MATERIALIZED VIEW IF NOT EXISTS dbunk_demo.events_to_rollups
TO dbunk_demo.metric_rollups
AS
SELECT
  toDate(event_time) AS rollup_date,
  account_id,
  event_name,
  count() AS events,
  sum(revenue) AS revenue,
  max(ifNull(duration_ms, 0)) AS max_duration_ms
FROM dbunk_demo.events
GROUP BY rollup_date, account_id, event_name;

CREATE TABLE IF NOT EXISTS dbunk_demo.schema_edge_cases
(
  id UInt64,
  nullable_text Nullable(String),
  empty_array Array(String),
  nested_scores Array(Tuple(label String, score Float64)),
  labels Map(String, String),
  maybe_number Nullable(Decimal(18, 6)),
  created_at DateTime64(6, 'UTC')
)
ENGINE = MergeTree
ORDER BY id;

INSERT INTO dbunk_demo.accounts VALUES
  ('00000000-0000-4000-8000-000000000001', 'acct_acme', 'Acme Analytics', 'enterprise', 'us-east', ['priority','sso'], {'timezone':'America/New_York','retention':'730'}, 50000.00, '2026-01-02 10:00:00.000'),
  ('00000000-0000-4000-8000-000000000002', 'acct_zen', 'Zenith Retail', 'pro', 'ap-south', ['retail','overdue'], {'timezone':'Asia/Kolkata','retention':'90'}, 2500.00, '2026-02-08 08:15:00.000'),
  ('00000000-0000-4000-8000-000000000003', 'acct_nulls', 'Null Island Labs', 'free', 'global', [], {'timezone':'','retention':'7'}, 0.00, '2026-02-20 16:45:00.000'),
  ('00000000-0000-4000-8000-000000000004', 'acct_suspended', 'Suspended Co', 'suspended', 'eu-west', ['risk','readonly'], {'locked_reason':'payment_failed'}, 100.00, '2026-01-20 09:00:00.000');

INSERT INTO dbunk_demo.sessions VALUES
  ('40000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000001', 101, '2026-02-25 10:00:00.000', '2026-02-25 10:08:33.000', 'Dbunk Desktop/0.1 macOS', '203.0.113.10', 'US', false),
  ('40000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000002', 201, '2026-02-27 11:30:00.000', NULL, 'Mozilla/5.0 extremely long agent string for grid wrapping checks', '198.51.100.25', 'IN', false),
  ('40000000-0000-4000-8000-000000000003', '00000000-0000-4000-8000-000000000003', 301, '2026-02-28 05:00:00.000', NULL, 'SyntheticBot/1.0', '192.0.2.44', 'ZZ', true);

INSERT INTO dbunk_demo.events (event_id, account_id, session_id, event_name, event_time, revenue, duration_ms, attributes, raw_json, version) VALUES
  ('50000000-0000-4000-8000-000000000001', '00000000-0000-4000-8000-000000000001', '40000000-0000-4000-8000-000000000001', 'workspace.opened', '2026-02-25 10:00:01.250', 0.00, 151, {'route':'/tables','panel':'schema'}, '{"route":"/tables","duration_ms":151}', 1),
  ('50000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000001', '40000000-0000-4000-8000-000000000001', 'query.finished', '2026-02-25 10:01:13.100', 0.00, 820, {'rows':'125000','format':'grid'}, '{"rows":125000,"warnings":[]}', 1),
  ('50000000-0000-4000-8000-000000000003', '00000000-0000-4000-8000-000000000002', '40000000-0000-4000-8000-000000000002', 'purchase.completed', '2026-02-27 11:32:45.777', 149.99, NULL, {'currency':'USD','coupon':'SPRING'}, '{"currency":"USD","items":2,"coupon":"SPRING"}', 1),
  ('50000000-0000-4000-8000-000000000004', '00000000-0000-4000-8000-000000000004', '40000000-0000-4000-8000-000000000002', 'connection.rejected', '2026-02-27 11:35:00.000', 0.00, 5, {'reason':'suspended_account','retryable':'false'}, '{"reason":"suspended_account","retryable":false}', 1),
  ('50000000-0000-4000-8000-000000000004', '00000000-0000-4000-8000-000000000004', '40000000-0000-4000-8000-000000000002', 'connection.rejected', '2026-02-27 11:35:00.000', 0.00, 4, {'reason':'suspended_account','retryable':'false','deduped':'true'}, '{"reason":"suspended_account","retryable":false,"deduped":true}', 2);

INSERT INTO dbunk_demo.event_items VALUES
  ('50000000-0000-4000-8000-000000000003', 'sku-pro-seat', 2, 49.00, [0.00]),
  ('50000000-0000-4000-8000-000000000003', 'sku-export-pack', 1, 51.99, [10.00, 5.00]);

INSERT INTO dbunk_demo.schema_edge_cases VALUES
  (1, NULL, [], [('quality', 0.98), ('risk', -1.25)], {'empty':'','unicode':'cafe'}, NULL, '2026-02-28 05:00:00.123456'),
  (2, repeat('long cell ', 40), ['one', 'two'], [], {'jsonish':'{"not":"typed"}'}, 123456789.123456, '2026-02-28 05:01:00.654321');
