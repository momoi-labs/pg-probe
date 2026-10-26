---
"pg-probe": minor
---

`pg-probe top` now uses the full width of the terminal, with no empty columns
at its sides, and a page that fits uses the full width as well. An empty list
says so in the middle of the screen. The header shows the time of the last
refresh in place of the spinner, and `‖ paused` with the time of the data
while paused. In `top` and in the capture, the version shows apart from the
name, on a gray fill.
