A user's hook command that takes no `{path}` still reads the call's arguments on its standard input, and so the name of a secret file the call's question is about, and may open it itself before the person's answer.

- Area: cineris-core
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **A hook is held back from a secret file only when `{path}` would hand it the file.** Since the fixes to step 28 (finding 14 of its review, the owner's choice), a `tool.before` hook command whose `{path}` would be filled with a file the engine asks about for what it is (it looks secret, it is protected and the call writes it, or a path rule asks or denies) runs only after the person's yes ([12-extensions.md](../architecture/12-extensions.md), [security.md](../security.md) §3). Every hook command is also given the call, its arguments included, as JSON on its standard input, so a user's hook that takes no `{path}` learns the file's name, and can read the file itself before the question is answered, as a hook may read the workspace; one given the network (`network = true`) can send it on. The owner's choice was about `{path}`, so this is left as it is. To settle: hold back any hook command of a call that names such a file, or give such a hook the call with the file's name left out until the yes, or keep a user's `tool.before` hook from reading the files held back; decide which with the owner. | Step 32 (the hardening pass), found by the fixes to step 28 ([steps/step-28-fixes/answered.md](../steps/step-28-fixes/answered.md)) | Before the first release |
