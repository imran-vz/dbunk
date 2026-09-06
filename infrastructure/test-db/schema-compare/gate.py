#!/usr/bin/env python3
"""Opt-in Plan 021 capture experiments, confined to a fresh no-network container.

Run: python3 infrastructure/test-db/schema-compare/gate.py
No DSN or existing container can be supplied. Successful assertions establish
the documented conservative subset, not general expression comparability.
"""

import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import tempfile
import time
import uuid

import discovery


ROOT = Path(__file__).resolve().parent
IMAGE = "dbunk-schema-compare-gate:local"


def docker(*args, timeout=30):
    return subprocess.run(
        ["docker", *args], check=True, text=True, capture_output=True, timeout=timeout
    ).stdout.strip()


class Session:
    """One persistent backend; echo acknowledgements order concurrent actions."""

    def __init__(self, container):
        self.process = subprocess.Popen(
            # Merge inside the container: Docker's separate stdout/stderr
            # streams can otherwise deliver an error after the echo marker.
            ["docker", "exec", "-i", container, "sh", "-c",
             "exec psql -X -qAt -U postgres -d schema_compare_gate "
             "--set=VERBOSITY=verbose 2>&1"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, bufsize=1,
        )
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        for line in self.process.stdout:
            self.lines.put(line.rstrip("\n"))
        self.lines.put(None)

    def sql(self, sql, error=None):
        marker = "dbunk_done_" + uuid.uuid4().hex
        self.process.stdin.write(sql + "\n\\echo " + marker + "\n")
        self.process.stdin.flush()
        lines = []
        deadline = time.monotonic() + 15
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("fixture command exceeded 15-second client deadline")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as timeout_error:
                raise TimeoutError(
                    "fixture command exceeded 15-second client deadline"
                ) from timeout_error
            if line == marker:
                break
            if line is None:
                raise RuntimeError("fixture backend closed: " + "\n".join(lines))
            lines.append(line)
        output = "\n".join(lines)
        if error:
            check("ERROR:  " + error in output, output)
        else:
            check("ERROR:" not in output and "FATAL:" not in output, output)
        return output

    def begin(self, locks=""):
        self.sql("""
            BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
            SET LOCAL search_path = pg_catalog;
            SET LOCAL timezone = 'UTC';
            SET LOCAL datestyle = 'ISO, YMD';
            SET LOCAL intervalstyle = 'postgres';
            SET LOCAL extra_float_digits = 3;
            SET LOCAL standard_conforming_strings = on;
        """ + locks)

    def close(self):
        try:
            if self.process.poll() is None:
                self.process.stdin.write("\\q\n")
                self.process.stdin.flush()
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        finally:
            self.process.stdin.close()
        self.reader.join(timeout=5)
        self.process.stdout.close()


def check(condition, detail):
    if not condition:
        raise AssertionError(detail)


# Direct catalog facts use the transaction snapshot; renderer calls may consult
# fresh syscache/relcache entries. Keep both in the same returned observation.
CHECK_FACTS = """
    SELECT a.attname, pg_get_expr(c.conbin, c.conrelid, false)
    FROM pg_catalog.pg_constraint c
    JOIN pg_catalog.pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = 2
    JOIN pg_catalog.pg_class r ON r.oid = c.conrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'orders' AND c.conname = 'positive';
"""
INDEX_FACTS = """
    SELECT r.relname, pg_get_indexdef(i.indexrelid, 0, false),
           pg_get_expr(i.indexprs, i.indrelid, false),
           pg_get_expr(i.indpred, i.indrelid, false)
    FROM pg_catalog.pg_index i
    JOIN pg_catalog.pg_class r ON r.oid = i.indexrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'orders_expression';
"""
DEFAULT_FACTS = """
    SELECT a.attname, pg_get_expr(d.adbin, d.adrelid, false)
    FROM pg_catalog.pg_attrdef d
    JOIN pg_catalog.pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum
    JOIN pg_catalog.pg_class r ON r.oid = d.adrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'orders' ORDER BY a.attnum;
"""
PROBE = """
    SELECT pg_get_expr(d.adbin, d.adrelid, false)
    FROM pg_catalog.pg_attrdef d
    JOIN pg_catalog.pg_class r ON r.oid = d.adrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'output_probe';
"""
HIDDEN_DEFAULT = """
    SELECT pg_get_expr(d.adbin, d.adrelid, false)
    FROM pg_catalog.pg_attrdef d
    JOIN pg_catalog.pg_class r ON r.oid = d.adrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'hidden_dependency';
"""
ENDPOINT_CHECK_FACTS = """
    SELECT n.nspname, r.relname, a.attname,
           pg_get_expr(c.conbin, c.conrelid, false)
    FROM pg_catalog.pg_constraint c
    JOIN pg_catalog.pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = 2
    JOIN pg_catalog.pg_class r ON r.oid = c.conrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname IN ('source', 'target') AND r.relname = 'orders'
      AND c.conname = 'positive'
    ORDER BY n.nspname;
"""
INDEX_IDENTITY_FACTS = """
    SELECT i.indexrelid, r.relname,
           pg_get_expr(i.indexprs, i.indrelid, false),
           pg_get_expr(i.indpred, i.indrelid, false)
    FROM pg_catalog.pg_index i
    JOIN pg_catalog.pg_class r ON r.oid = i.indexrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
    WHERE n.nspname = 'source' AND r.relname = 'orders_expression';
"""


def experiments(a, b):
    observations = {}

    a.begin()
    observations["baseline_check"] = a.sql(CHECK_FACTS)
    observations["baseline_defaults"] = a.sql(DEFAULT_FACTS)
    observations["baseline_index"] = a.sql(INDEX_FACTS)
    check("quantity|(quantity > 0)" in observations["baseline_check"], observations)
    a.sql("ROLLBACK;")

    # Establish the snapshot without touching the table through a renderer.
    a.begin()
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    b.sql("ALTER TABLE source.orders RENAME COLUMN quantity TO amount;")
    observations["column_rename_check"] = a.sql(CHECK_FACTS)
    observations["column_rename_defaults"] = a.sql(DEFAULT_FACTS)
    observations["column_rename_index"] = a.sql(INDEX_FACTS)
    check("quantity|(amount > 0)" in observations["column_rename_check"], observations)
    check(observations["column_rename_defaults"] == observations["baseline_defaults"],
          observations)
    check("orders_expression|CREATE INDEX orders_expression ON "
          "source.orders USING btree (((amount + 1))) INCLUDE (label) "
          "WHERE (amount > 0)|(amount + 1)|(amount > 0)" in
          observations["column_rename_index"], observations)
    a.sql("ROLLBACK;")
    b.sql("ALTER TABLE source.orders RENAME COLUMN amount TO quantity;")

    a.begin()
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    b.sql("ALTER TABLE source.orders RENAME TO renamed_orders;")
    observations["table_rename_check"] = a.sql(CHECK_FACTS)
    observations["table_rename_defaults"] = a.sql(DEFAULT_FACTS)
    observations["table_rename_index"] = a.sql(INDEX_FACTS)
    check(observations["table_rename_check"] == observations["baseline_check"],
          observations)
    check(observations["table_rename_defaults"] == observations["baseline_defaults"],
          observations)
    check("source.renamed_orders" in observations["table_rename_index"], observations)
    check("|(quantity + 1)|(quantity > 0)" in
          observations["table_rename_index"], observations)
    a.sql("ROLLBACK;")
    b.sql("ALTER TABLE source.renamed_orders RENAME TO orders;")

    # Taking the lock after establishing the snapshot does not repair names.
    a.begin()
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    b.sql("ALTER TABLE source.orders RENAME COLUMN quantity TO amount;")
    a.sql("LOCK TABLE ONLY source.orders IN ACCESS SHARE MODE;")
    observations["late_lock_mixed"] = a.sql(CHECK_FACTS)
    check("quantity|(amount > 0)" in observations["late_lock_mixed"], observations)
    a.sql("ROLLBACK;")
    b.sql("ALTER TABLE source.orders RENAME COLUMN amount TO quantity;")

    # Both same-database endpoints are locked before the first SELECT.
    a.begin("LOCK TABLE ONLY source.orders, ONLY target.orders, "
            "ONLY source.hidden_dependency IN ACCESS SHARE MODE;")
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    observations["source_rename_blocked"] = b.sql(
        "ALTER TABLE source.orders RENAME COLUMN quantity TO amount;", error="55P03")
    observations["target_rename_blocked"] = b.sql(
        "ALTER TABLE target.orders RENAME COLUMN quantity TO amount;", error="55P03")
    observations["locked_endpoint_checks"] = a.sql(ENDPOINT_CHECK_FACTS)
    observations["locked_check"] = a.sql(CHECK_FACTS)
    check(observations["locked_endpoint_checks"] ==
          "source|orders|quantity|(quantity > 0)\n"
          "target|orders|quantity|(quantity > 0)", observations)
    check(observations["locked_check"] == observations["baseline_check"], observations)
    observations["hidden_dependency_before"] = a.sql(HIDDEN_DEFAULT)
    observations["hidden_dependency_edges"] = a.sql("""
        SELECT dep.refclassid::regclass, dep.deptype
        FROM pg_catalog.pg_depend dep
        JOIN pg_catalog.pg_attrdef d
          ON dep.classid = 'pg_catalog.pg_attrdef'::regclass AND dep.objid = d.oid
        WHERE d.adrelid = 'source.hidden_dependency'::regclass
        ORDER BY dep.refclassid, dep.deptype;
    """)
    check(observations["hidden_dependency_edges"] == "pg_class|a", observations)

    # ACCESS SHARE protects the table's descriptor but not referenced objects.
    b.sql("ALTER FUNCTION external.identity_int(integer) RENAME TO renamed_identity;")
    b.sql("ALTER SEQUENCE external.serial RENAME TO renamed_serial;")
    b.sql("ALTER TYPE external.mood RENAME VALUE 'calm' TO 'changed';")
    observations["locked_dependency_rename"] = a.sql(DEFAULT_FACTS)
    check("id|7\nlabel|'source.literal ''quoted'''::text" in
          observations["locked_dependency_rename"], observations)
    check("renamed_identity" in observations["locked_dependency_rename"], observations)
    check("renamed_serial" in observations["locked_dependency_rename"], observations)
    check("changed" in observations["locked_dependency_rename"], observations)
    observations["hidden_dependency_after"] = a.sql(HIDDEN_DEFAULT)
    check("external.serial" in observations["hidden_dependency_before"], observations)
    check("external.renamed_serial" in observations["hidden_dependency_after"], observations)

    # Index rename does not need an exclusive lock on the parent table.
    b.sql("ALTER INDEX source.orders_expression RENAME TO renamed_index;")
    observations["locked_index_rename"] = a.sql(INDEX_FACTS)
    check("orders_expression|CREATE INDEX renamed_index" in
          observations["locked_index_rename"], observations)
    # Concurrent DROP can invalidate first, then wait for the old snapshot.
    observations["concurrent_index_drop_wait"] = b.sql(
        "DROP INDEX CONCURRENTLY source.renamed_index;", error="55P03")
    observations["snapshot_index_validity"] = a.sql(
        "SELECT indisvalid FROM pg_catalog.pg_index "
        "WHERE indexrelid IN (SELECT oid FROM pg_catalog.pg_class "
        "WHERE relname = 'orders_expression');")
    observations["current_index_validity"] = b.sql(
        "SELECT indisvalid FROM pg_catalog.pg_index "
        "WHERE indexrelid = 'source.renamed_index'::regclass;")
    check(observations["snapshot_index_validity"] == "t", observations)
    check(observations["current_index_validity"] == "f", observations)
    observations["locked_index_expressions"] = a.sql(INDEX_FACTS)
    a.sql("ROLLBACK;")
    b.sql("DROP INDEX source.renamed_index;")
    b.sql("CREATE INDEX CONCURRENTLY orders_expression ON source.orders ((quantity + 2));")
    b.sql("ALTER FUNCTION external.renamed_identity(integer) RENAME TO identity_int;")
    b.sql("ALTER SEQUENCE external.renamed_serial RENAME TO serial;")
    b.sql("ALTER TYPE external.mood RENAME VALUE 'changed' TO 'calm';")

    # Build the replacement before capture, then swap its identity while the
    # reader's locked repeatable-read snapshot remains open.
    b.sql("CREATE INDEX CONCURRENTLY orders_expression_replacement "
          "ON source.orders ((quantity + 3)) WHERE quantity > 0;")
    old_index_oid = b.sql("SELECT 'source.orders_expression'::regclass::oid;")
    replacement_index_oid = b.sql(
        "SELECT 'source.orders_expression_replacement'::regclass::oid;")
    a.begin("LOCK TABLE ONLY source.orders IN ACCESS SHARE MODE;")
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    b.sql("BEGIN; "
          "ALTER INDEX source.orders_expression RENAME TO orders_expression_retired; "
          "ALTER INDEX source.orders_expression_replacement RENAME TO orders_expression; "
          "COMMIT;")
    observations["replacement_snapshot_index"] = a.sql(INDEX_IDENTITY_FACTS)
    observations["replacement_current_index"] = b.sql(INDEX_IDENTITY_FACTS)
    check(observations["replacement_snapshot_index"] ==
          old_index_oid + "|orders_expression|(quantity + 2)|",
          observations)
    check(observations["replacement_current_index"] ==
          replacement_index_oid + "|orders_expression|(quantity + 3)|(quantity > 0)",
          observations)
    check(old_index_oid != replacement_index_oid, observations)
    a.sql("ROLLBACK;")
    b.sql("DROP INDEX CONCURRENTLY source.orders_expression_retired;")

    a.begin()
    observations["strong_lock_read_only"] = a.sql(
        "LOCK TABLE ONLY source.orders IN SHARE UPDATE EXCLUSIVE MODE;")
    a.sql("SELECT count(*) FROM pg_catalog.pg_class;")
    observations["strong_lock_index_rename"] = b.sql(
        "ALTER INDEX source.orders_expression RENAME TO strong_lock_index;")
    observations["strong_lock_index_mixed"] = a.sql(INDEX_FACTS)
    check("orders_expression|CREATE INDEX strong_lock_index" in
          observations["strong_lock_index_mixed"], observations)
    a.sql("ROLLBACK;")
    b.sql("ALTER INDEX source.strong_lock_index RENAME TO orders_expression;")
    a.sql("SET ROLE catalog_reader;")
    a.begin("LOCK TABLE ONLY source.orders, ONLY target.orders IN ACCESS SHARE MODE;")
    observations["select_privilege_lock"] = a.sql(CHECK_FACTS)
    a.sql("ROLLBACK;")
    a.sql("BEGIN;")
    observations["strong_lock_permission"] = a.sql(
        "LOCK TABLE ONLY source.orders IN SHARE UPDATE EXCLUSIVE MODE;", error="42501")
    a.sql("ROLLBACK; RESET ROLE;")

    # Successful NULL rendering must never be interpreted as an absent fact.
    b.sql("CREATE TABLE source.recreated (value integer DEFAULT 1 CHECK (value > 0));")
    a.begin()
    oid = a.sql("SELECT 'source.recreated'::regclass::oid;")
    b.sql("DROP TABLE source.recreated; CREATE TABLE source.recreated (other text);")
    observations["drop_recreate"] = a.sql(
        "SELECT conname, pg_get_expr(conbin, conrelid, false) IS NULL "
        f"FROM pg_catalog.pg_constraint WHERE conrelid = {int(oid)};")
    check("recreated_value_check|t" in observations["drop_recreate"], observations)
    a.sql("ROLLBACK;")

    a.begin()
    observations["output_notice"] = a.sql(PROBE)
    check("dbunk output probe reached" in observations["output_notice"], observations)
    a.sql("SET LOCAL dbunk.output_probe = 'error';")
    observations["output_error"] = a.sql(PROBE, error="22000")
    a.sql("ROLLBACK;")
    a.begin()
    a.sql("SET LOCAL statement_timeout = '100ms'; SET LOCAL dbunk.output_probe = 'timeout';")
    started = time.monotonic()
    observations["output_timeout"] = a.sql(PROBE, error="57014")
    observations["output_timeout_seconds"] = round(time.monotonic() - started, 3)
    a.sql("ROLLBACK;")

    # MATERIALIZED evaluates the renderer once. Only the small status and NULL
    # cross the protocol; server heap inside support code remains unbounded.
    a.begin()
    a.sql("SET LOCAL dbunk.output_probe = 'oversize';")
    guarded = """
        WITH rendered AS MATERIALIZED (
    """ + PROBE.rstrip().removesuffix(";").replace(
        "SELECT pg_get_expr(d.adbin, d.adrelid, false)",
        "SELECT pg_get_expr(d.adbin, d.adrelid, false) AS value") + """
        ) SELECT octet_length(value) > 262144 AS limit_exceeded,
                 CASE WHEN octet_length(value) <= 262144 THEN value END
          FROM rendered;
    """
    observations["oversize_guard"] = a.sql(guarded)
    check("t|" in observations["oversize_guard"], observations)
    check(len(observations["oversize_guard"].encode()) < 1024, observations)
    check(observations["oversize_guard"].count("probe reached") == 1, observations)
    a.sql("ROLLBACK;")
    return observations


def cleanup(sessions, name, container_created):
    """Attempt every cleanup step and return failures without masking a test error."""
    failures = []
    for session in sessions:
        try:
            session.close()
        except BaseException as error:
            failures.append("session close: " + repr(error))
    if container_created:
        try:
            docker("rm", "-f", name)
        except BaseException as error:
            failures.append("container removal: " + repr(error))
    return failures


def report_cleanup_failures(failures):
    for failure in failures:
        print("Cleanup failure: " + failure, file=sys.stderr, flush=True)


def main():
    subprocess.run(["docker", "build", "-t", IMAGE, str(ROOT)], check=True, timeout=600)
    name = "dbunk-schema-compare-gate-" + uuid.uuid4().hex[:12]
    print("Disposable fixture: " + name, flush=True)
    sessions = []
    container_created = False
    try:
        docker("create", "--name", name, "--network", "none",
               "--memory", "512m", "--cpus", "2", "--pids-limit", "100",
               "--tmpfs", "/var/lib/postgresql/data", "-e",
               "POSTGRES_HOST_AUTH_METHOD=trust", "-e",
               "POSTGRES_DB=schema_compare_gate", IMAGE)
        container_created = True
        docker("start", name)
        deadline = time.monotonic() + 30
        while True:
            try:
                # The entrypoint starts a temporary server during init. Wait
                # for the final ready message before opening persistent clients.
                logs = subprocess.run(["docker", "logs", name], check=True,
                                      capture_output=True, text=True, timeout=5)
                if "PostgreSQL init process complete" in logs.stdout + logs.stderr:
                    docker("exec", name, "pg_isready", "-U", "postgres")
                    break
            except subprocess.CalledProcessError:
                pass
            if time.monotonic() > deadline:
                raise TimeoutError("disposable PostgreSQL startup")
            time.sleep(0.1)
        for _ in range(2):
            session = Session(name)
            sessions.append(session)
            session.sql("SET statement_timeout = '5s'; SET lock_timeout = '500ms';")
        a, b = sessions
        a.sql(ROOT.joinpath("fixture.sql").read_text())
        version = a.sql("SELECT version();")
        check(a.sql("SHOW server_version_num;").startswith("16"), version)
        result = experiments(a, b)
        result['discovery'] = discovery.run(a, b)
        with tempfile.TemporaryDirectory(prefix='dbunk-schema-compare-expression-') as directory:
            probe = str(Path(directory) / 'probe')
            subprocess.run(['rustc', '--edition', '2021', str(ROOT / 'expression_probe.rs'),
                            '-o', probe], check=True, timeout=60)
            def classify(value, *columns):
                return subprocess.run([probe, *columns], input=value, text=True,
                                      capture_output=True, check=True, timeout=5).stdout.strip()
            check(classify('7') == 'comparable', result)
            check(classify(result['baseline_check'].split('|', 1)[1], 'quantity')
                  == 'comparable', result)
            check(classify('(quantity + 1)', 'quantity') == 'comparable', result)
            check(classify(result['hidden_dependency_before']) == 'notComparable', result)
            check(classify(result['hidden_dependency_after']) == 'notComparable', result)
            check(classify("external.identity_int(7)") == 'notComparable', result)
            result['positiveExpressionSubset'] = 'passed'
        print(json.dumps({
            "server": version,
            "reproductions": "passed",
            "captureGate": "conservativeSubset",
            "reason": "Only the native positive scalar grammar is comparable; "
                      "unknown expressions retain notComparable coverage.",
            "observations": result,
        }, indent=2))
    except BaseException:
        report_cleanup_failures(cleanup(sessions, name, container_created))
        raise
    cleanup_failures = cleanup(sessions, name, container_created)
    report_cleanup_failures(cleanup_failures)
    if cleanup_failures:
        raise RuntimeError("disposable fixture cleanup failed")


if __name__ == "__main__":
    main()
