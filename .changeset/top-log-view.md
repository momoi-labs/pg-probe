---
"pg-probe": minor
---

`pg-probe top` has a log view on `F6` or `6`. On the database host, it shows the lines of the server's csvlog as they arrive, like `tail -f`, and `e`, `t` and `c` keep only the errors, the temp files or the checkpoints. Each filter shows beside the list a summary since `top` started, which `←` and `→` resize: the errors with the most lines per user and database, with the count of the last 5 minutes; the statements that wrote the most temp file bytes; and what started the checkpoints, with p95 and p99 of buffers, write and sync time. `Enter` shows every field of the selected line with its statement, and `/` searches the lines. When `logging_collector`, `log_temp_files` or `log_checkpoints` is off, the view says what is missing and `S` opens the settings view at that setting. It reads the file from the local disk, at most 8 MiB per refresh, and writes none of it. It works from PostgreSQL 9.3.
