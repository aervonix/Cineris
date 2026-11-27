An extension's `cineris.progress` lines are added to its tool's output only when the call ends, and a hook's or a command's are kept nowhere.

- Area: cineris-core
- Owner: step 30

| Item | Who | When |
|---|---|---|
| **Progress that is not shown while the call runs.** `cineris.progress(text)` keeps each line in the call's host (`ext::host::CallHost::progress`, cut at 200 bytes by `cineris_ext::host::PROGRESS_MAX`, the last 20 kept), and the tool's call adds them to its output after the script has returned (`ext::tools::LoadedExtension::call_tool`), so a front end sees a long call's progress only once it is over. A hook's and a command's host keep their lines in the same way, and nothing reads them. The design page said progress goes to the front ends, and step 28's settled point said "200 characters, the latest kept"; the review of step 28's close found both, and [architecture/12-extensions.md](../architecture/12-extensions.md) now says what the code does. **Recommendation:** hand the call's host the tool's output, so that a line reaches the front ends as it is made (the transcript keeps the tool's output as it does a command's), and show a hook's or a command's line as a notice is shown, or refuse `progress` there with an error that says why. Step 30's MCP client needs the same for a server's progress notifications. | Step 30 (MCP client), which streams a server's progress the same way | Before the first release |
