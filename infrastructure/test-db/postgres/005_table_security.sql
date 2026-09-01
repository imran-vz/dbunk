\set ON_ERROR_STOP on

-- Table-scoped security and routine fixtures for Plan 016 live tests.
-- Objects here are additive; the Plan 013 fixture in 004 is unchanged.

CREATE ROLE lifecycle_reader NOLOGIN;
GRANT SELECT ON lifecycle.orders TO lifecycle_reader;

CREATE FUNCTION lifecycle.touch_orders()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  NEW.retry_count := COALESCE(NEW.retry_count, 1);
  RETURN NEW;
END;
$$;

CREATE TRIGGER orders_touch
  BEFORE UPDATE ON lifecycle.orders
  FOR EACH ROW
  EXECUTE FUNCTION lifecycle.touch_orders();

CREATE TABLE lifecycle.tenant_rows (
  id serial PRIMARY KEY,
  tenant text NOT NULL,
  note text
);
ALTER TABLE lifecycle.tenant_rows ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON lifecycle.tenant_rows
  FOR SELECT TO PUBLIC
  USING (tenant = current_setting('app.tenant', true));

CREATE FUNCTION lifecycle.order_total(order_id integer)
RETURNS numeric
LANGUAGE plpgsql
STABLE
AS $$
BEGIN
  RETURN (SELECT amount FROM lifecycle.orders WHERE id = order_id);
END;
$$;
