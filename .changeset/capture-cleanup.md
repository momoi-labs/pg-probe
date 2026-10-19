---
"pg-probe": minor
---

`pg-probe capture --cleanup` deletes the `.sqlite.partial` files left by
earlier captures. It lists them and asks to confirm: `y` deletes them and the
capture continues; any other answer aborts with exit code 2 and deletes
nothing.
