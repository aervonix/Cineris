Under Windows Terminal the viewer and the full screen have no mouse: the terminal is not asked for it, because a pseudo-console would give its reports to the program as typed keys.

- Area: cortex-term
- Owner: step 33

| Item | Who | When |
|---|---|---|
| **No mouse under Windows Terminal.** Measured in step 25 on a hosted runner: with modes 1000 and 1006 written, ConPTY passes them on to the terminal and then gives the terminal's reports to the program as typed keys (it has asked the terminal for win32 input records, and reads a plain sequence after an escape as keys, `docs/conpty.md`), so a turn of the wheel would be typed into the prompt; with the console's mouse input alone, it asks the terminal for nothing. So on Windows the terminal is asked for nothing (`Capabilities::vt_mouse`), keeps its own wheel and selection, and the keys do what the mouse does; the old console host, with no pseudo-console, gives the program its mouse records. What is not known: whether a newer ConPTY (the one Windows Terminal ships with its own `OpenConsole`) turns the reports into console mouse records when the program uses the console's mouse input, and what Windows Terminal itself sends. Recommendation: on a real Windows Terminal, try the console's mouse input with the VT input mode off while the mouse is on, and the bundled `OpenConsole`; turn the mouse on there only once a test shows the wheel arriving as a wheel and never as typed keys. | Step 33, after the Windows Terminal checklist of step 25 | Before the first release |
