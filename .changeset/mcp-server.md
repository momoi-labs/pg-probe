---
"pg-probe": minor
---

`pg-probe mcp` lets an AI agent such as Claude Code or Codex query the
artifacts in a directory, instead of reading the console panel by panel. It is
a local MCP server over stdio with four tools: list the snapshots, describe
each table and column, run a read-only SQL query, and export a query to CSV.
SQL on an artifact can now compute percentiles and use math functions.
