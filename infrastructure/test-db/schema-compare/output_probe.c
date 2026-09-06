/* Fixture-only base type: prove that catalog rendering reaches type output
 * code, including its notices, errors, timeouts, and oversized output. */
#include "postgres.h"
#include "fmgr.h"
#include "utils/builtins.h"
#include "utils/guc.h"
#include "miscadmin.h"

PG_MODULE_MAGIC;
PG_FUNCTION_INFO_V1(dbunk_probe_in);
PG_FUNCTION_INFO_V1(dbunk_probe_out);

Datum
dbunk_probe_in(PG_FUNCTION_ARGS)
{
    PG_RETURN_INT32(pg_strtoint32(PG_GETARG_CSTRING(0)));
}

Datum
dbunk_probe_out(PG_FUNCTION_ARGS)
{
    const char *mode = GetConfigOptionByName("dbunk.output_probe", NULL, true);

    ereport(NOTICE, (errmsg("dbunk output probe reached")));
    if (mode != NULL && strcmp(mode, "error") == 0)
        ereport(ERROR, (errcode(ERRCODE_DATA_EXCEPTION),
                        errmsg("dbunk output probe controlled error")));
    if (mode != NULL && strcmp(mode, "timeout") == 0)
    {
        for (;;)
        {
            CHECK_FOR_INTERRUPTS();
            pg_usleep(1000L);
        }
    }
    if (mode != NULL && strcmp(mode, "oversize") == 0)
    {
        char *result = palloc(262146);
        memset(result, 'x', 262145);
        result[262145] = '\0';
        PG_RETURN_CSTRING(result);
    }
    PG_RETURN_CSTRING(psprintf("%d", PG_GETARG_INT32(0)));
}
