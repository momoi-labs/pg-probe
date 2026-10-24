---
"pg-probe": patch
---

The release checks artifacts made in Rocky Linux 9 with the host's sqlite3,
since Rocky's is too old for their STRICT tables. v0.2.1 was tagged but not
published because that check failed; this release ships the same changes.
