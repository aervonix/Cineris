A mouse report the terminal sent just before the mouse was turned off can arrive afterwards and be read as text.

- Area: cineris-term
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **Reports in flight become text.** The terminal layer turns the mouse off with the alternate screen and at every way out (since step 25), but a report already on its way is still delivered. Three ways it shows, found by the review of step 25: on quitting, raw mode is left with `TCSADRAIN`, so a report that arrives after (the wheel's momentum on a trackpad, an SSH connection's delay) reaches the shell as `^[[<65;…M`; `alternate(false)` stops reading the older form (mode 1000's `CSI M` and three bytes) at once, so one of those in flight types its three bytes into the editor (terminals that know mode 1006 send the SGR form, which is always read); and a partial sequence is given up on after 50 ms without first trying another read (`Terminal::next`), so a report split across two reads by a slow frame becomes `Alt+[` and text. None loses a key the person typed. Recommendation: after turning the mouse off, read and drop mouse reports for a short time (50 ms) before the main screen's keys count; leave raw mode with `TCSAFLUSH` when the mouse was on in this run; and read once more before giving up on a partial sequence. | Step 33 (terminal hardening) | Before the first release |
