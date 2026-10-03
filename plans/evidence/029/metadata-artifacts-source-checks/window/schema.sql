-- Relation-oriented reconstruction; not a canonical database backup.
-- No table data is exported; materialized-view SQL preserves its captured populated state.
-- Standalone routines, types, domains, extensions and other non-relation objects are not included.
-- Relations are ordered by schema/name, not dependency order; referenced objects may need to exist first.
-- Ownership and privileges are not reconstructed.
-- Triggers, non-view rules, row-security settings and policies are not reconstructed.
-- Physical storage settings and ordinary table inheritance are not fully reconstructed.
-- Foreign-table SQL is a reconstruction, not verified remote compatibility. Foreign servers and user mappings are prerequisites and are not exported; no foreign data is read.
-- Standalone sequences and current sequence values are not exported; defaults may reference existing sequences.
-- Object comments and materialized-view indexes are not reconstructed.

CREATE SCHEMA IF NOT EXISTS "native_map_window_20261003";

CREATE TABLE "native_map_window_20261003"."child" (
  "id" integer NOT NULL,
  "tenant" integer NOT NULL,
  "parent_id" integer NOT NULL,
  "note" text,
  CONSTRAINT "child_parent_composite" FOREIGN KEY (tenant, parent_id) REFERENCES native_map_window_20261003.parent(tenant, id),
  CONSTRAINT "child_pkey" PRIMARY KEY (id)
);

CREATE TABLE "native_map_window_20261003"."links" (
  "child_id" integer NOT NULL,
  "other_id" integer NOT NULL,
  CONSTRAINT "links_child_id_fkey" FOREIGN KEY (child_id) REFERENCES native_map_window_20261003.child(id),
  CONSTRAINT "links_other_id_fkey" FOREIGN KEY (other_id) REFERENCES native_map_window_20261003_external.other_parent(id),
  CONSTRAINT "links_pkey" PRIMARY KEY (child_id, other_id)
);

CREATE TABLE "native_map_window_20261003"."parent" (
  "tenant" integer NOT NULL,
  "id" integer NOT NULL,
  "name" text,
  CONSTRAINT "parent_pkey" PRIMARY KEY (tenant, id)
);

