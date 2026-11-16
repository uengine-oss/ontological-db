/*
 * og_mold_trace — write every SQL statement a backend runs to a file, nested ones included.
 *
 * Test tooling for the generated-SQL mold (tests/seam/). It is not part of the engine and is
 * never installed outside the throwaway cluster tests/seam/capture.sh makes.
 *
 * A statement the executor starts (SELECT, INSERT, UPDATE, DELETE — from a client, from SPI,
 * from PL/pgSQL, from a non-inlined SQL function) and a utility statement (CREATE, ALTER, SET,
 * DO …) each become one line:
 *
 *     <depth> <kind> <text>
 *
 * depth  0 for a statement the client sent, 1 for one run while a depth-0 statement was
 *        executing (the engine's SPI calls), and so on.
 * kind   E executor statement, U utility statement, X an error or warning that was reported
 *        (sqlstate and message), L a LOG message (the engine's own log! lines).
 * text   the statement's own slice of the query string (a multi-statement string is split the
 *        way PostgreSQL splits it), with \ newline CR tab written as \\ \n \r \t.
 *
 * Nothing is written while an extension script runs (CREATE EXTENSION), or when
 * og_mold_trace.file is empty. Loaded with session_preload_libraries; the file is chosen per
 * connection (PGOPTIONS='-c og_mold_trace.file=…'). The setting is USERSET so that a test which
 * reconnects as an ordinary role keeps being traced — which is also why this belongs only in a
 * throwaway cluster: any role can make the server append to any path it can write.
 *
 * PostgreSQL 16 and 17 (the hook signatures used here changed in 18).
 */
#include "postgres.h"

#include <stdio.h>
#include <string.h>

#include "commands/extension.h"
#include "executor/executor.h"
#include "fmgr.h"
#include "miscadmin.h"
#include "tcop/utility.h"
#include "utils/elog.h"
#include "utils/guc.h"

PG_MODULE_MAGIC;

void _PG_init(void);

static char *trace_path = NULL;
static FILE *trace_fp = NULL;
static char trace_open_path[MAXPGPATH];
static int nesting = 0;

static ExecutorStart_hook_type prev_ExecutorStart = NULL;
static ExecutorRun_hook_type prev_ExecutorRun = NULL;
static ExecutorFinish_hook_type prev_ExecutorFinish = NULL;
static ProcessUtility_hook_type prev_ProcessUtility = NULL;
static emit_log_hook_type prev_emit_log_hook = NULL;

static FILE *
trace_out(void)
{
	if (trace_path == NULL || trace_path[0] == '\0' || creating_extension)
		return NULL;
	if (trace_fp != NULL && strcmp(trace_open_path, trace_path) == 0)
		return trace_fp;
	if (trace_fp != NULL)
		fclose(trace_fp);
	trace_fp = fopen(trace_path, "a");
	if (trace_fp == NULL)
	{
		trace_open_path[0] = '\0';
		return NULL;
	}
	strlcpy(trace_open_path, trace_path, sizeof(trace_open_path));
	return trace_fp;
}

static void
put_escaped(FILE *f, const char *s, int len)
{
	int i;

	for (i = 0; i < len && s[i] != '\0'; i++)
	{
		switch (s[i])
		{
			case '\\': fputs("\\\\", f); break;
			case '\n': fputs("\\n", f); break;
			case '\r': fputs("\\r", f); break;
			case '\t': fputs("\\t", f); break;
			default: fputc(s[i], f);
		}
	}
}

/* The statement's own slice of `text`, trimmed of surrounding blanks. */
static void
emit_stmt(char kind, const char *text, int location, int len)
{
	FILE *f = trace_out();
	int total;
	const char *s;

	if (f == NULL || text == NULL)
		return;
	total = (int) strlen(text);
	if (location < 0 || location > total)
		location = 0;
	s = text + location;
	if (len <= 0 || location + len > total)
		len = total - location;
	while (len > 0 && (*s == ' ' || *s == '\n' || *s == '\t' || *s == '\r'))
		s++, len--;
	while (len > 0 && (s[len - 1] == ' ' || s[len - 1] == '\n' || s[len - 1] == '\t' || s[len - 1] == '\r'))
		len--;
	fprintf(f, "%d %c ", nesting, kind);
	put_escaped(f, s, len);
	fputc('\n', f);
	fflush(f);
}

static void
trace_ExecutorStart(QueryDesc *queryDesc, int eflags)
{
	if (queryDesc->sourceText != NULL && !(eflags & EXEC_FLAG_EXPLAIN_ONLY))
		emit_stmt('E', queryDesc->sourceText,
				  queryDesc->plannedstmt ? queryDesc->plannedstmt->stmt_location : 0,
				  queryDesc->plannedstmt ? queryDesc->plannedstmt->stmt_len : 0);
	if (prev_ExecutorStart)
		prev_ExecutorStart(queryDesc, eflags);
	else
		standard_ExecutorStart(queryDesc, eflags);
}

static void
trace_ExecutorRun(QueryDesc *queryDesc, ScanDirection direction, uint64 count, bool execute_once)
{
	nesting++;
	PG_TRY();
	{
		if (prev_ExecutorRun)
			prev_ExecutorRun(queryDesc, direction, count, execute_once);
		else
			standard_ExecutorRun(queryDesc, direction, count, execute_once);
	}
	PG_FINALLY();
	{
		nesting--;
	}
	PG_END_TRY();
}

static void
trace_ExecutorFinish(QueryDesc *queryDesc)
{
	nesting++;
	PG_TRY();
	{
		if (prev_ExecutorFinish)
			prev_ExecutorFinish(queryDesc);
		else
			standard_ExecutorFinish(queryDesc);
	}
	PG_FINALLY();
	{
		nesting--;
	}
	PG_END_TRY();
}

static void
trace_ProcessUtility(PlannedStmt *pstmt, const char *queryString, bool readOnlyTree,
					 ProcessUtilityContext context, ParamListInfo params,
					 QueryEnvironment *queryEnv, DestReceiver *dest, QueryCompletion *qc)
{
	emit_stmt('U', queryString, pstmt->stmt_location, pstmt->stmt_len);
	nesting++;
	PG_TRY();
	{
		if (prev_ProcessUtility)
			prev_ProcessUtility(pstmt, queryString, readOnlyTree, context, params, queryEnv, dest, qc);
		else
			standard_ProcessUtility(pstmt, queryString, readOnlyTree, context, params, queryEnv, dest, qc);
	}
	PG_FINALLY();
	{
		nesting--;
	}
	PG_END_TRY();
}

static void
trace_emit_log(ErrorData *edata)
{
	FILE *f;

	if (prev_emit_log_hook)
		prev_emit_log_hook(edata);
	if (edata->elevel < WARNING && edata->elevel != LOG)
		return;
	f = trace_out();
	if (f == NULL || edata->message == NULL)
		return;
	if (edata->elevel == LOG)
		fprintf(f, "%d L ", nesting);
	else
		fprintf(f, "%d X %s ", nesting, unpack_sql_state(edata->sqlerrcode));
	put_escaped(f, edata->message, (int) strlen(edata->message));
	fputc('\n', f);
	fflush(f);
}

void
_PG_init(void)
{
	DefineCustomStringVariable("og_mold_trace.file",
							   "Append every statement this session runs to this file.",
							   NULL, &trace_path, "", PGC_USERSET, 0, NULL, NULL, NULL);
	MarkGUCPrefixReserved("og_mold_trace");

	prev_ExecutorStart = ExecutorStart_hook;
	ExecutorStart_hook = trace_ExecutorStart;
	prev_ExecutorRun = ExecutorRun_hook;
	ExecutorRun_hook = trace_ExecutorRun;
	prev_ExecutorFinish = ExecutorFinish_hook;
	ExecutorFinish_hook = trace_ExecutorFinish;
	prev_ProcessUtility = ProcessUtility_hook;
	ProcessUtility_hook = trace_ProcessUtility;
	prev_emit_log_hook = emit_log_hook;
	emit_log_hook = trace_emit_log;
}
