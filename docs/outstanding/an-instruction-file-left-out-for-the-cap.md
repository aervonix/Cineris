An instruction file left out of the prompt for `context.instructions_cap` is still listed in the epoch as given.

- Area: cortex-agent
- Owner: a later step on instruction files, with the owner for the format

| Item | Who | When |
|---|---|---|
| **An instruction file left out of the prompt for the cap is recorded as given.** When the instruction files fill `context.instructions_cap`, the files after are left out of section 4, and the user is warned, but the epoch's `instructions` still lists each with its hash (`cortex-agent`'s `context::instructions::discover`). The model is never told of such a file unless it changes; then the change notice holds it whole, up to the cap, outside the cap on the prompt. Found by the review of step 15 (its eighth note). Not fixed in its fixes: telling a file left out from one given needs the `epoch` entry to say which files it left out, an addition to the log's format, and a resumed or rewound session must know it from the log alone. To settle: mark a left-out file in the epoch's `instructions` (or leave it out of them and its change notices with it), so that a change is told without its content, as a file the cap keeps out. | A later step that touches instruction files, with the owner for the format | Before projects with large instruction files are common |
