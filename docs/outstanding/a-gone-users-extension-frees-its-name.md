A pinned extension of the user's whose folder is gone no longer takes its name, so a project's extension of that name, once approved, is used and decided under the user's own rules for it.

- Area: cineris-core
- Owner: step 32 (hardening pass)

| Item | Who | When |
|---|---|---|
| **What is open.** A project's extension whose name a folder of the user's takes is not used, whatever the trust question's answer, since rules name an extension's tools by its name and the user's rules for theirs must not reach a repository's ([12-extensions.md](../architecture/12-extensions.md), "As built in step 28's second pull request"). The names taken are counted from the user's folders alone (`ext::found::UserNames`). A user's extension that is pinned and whose folder is gone (the `Gone` standing, whose gates hold) takes no name, so a project's `tracker` beside the user's gone `tracker` is listed by the trust question and, once approved, loaded and decided under the user's `allow ext tracker/*`. It needs the user to approve the project's extension first, in words that name it as the project's. Found by the full review of step 28 (its pins area, lower confidence), and left for step 32 by the maintainers. **What would close it:** count a pinned name of the user's as taken, whether or not its folder is there, until the pin is taken away (`cineris ext remove`), as the gates of a gone one are held. | Step 32 (hardening pass) | Before the first release |
