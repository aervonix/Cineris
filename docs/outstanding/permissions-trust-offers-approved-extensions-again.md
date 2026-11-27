`cineris permissions trust` shows a project's extensions that the person's signed entry already approves and offers them again, since a subcommand checks nothing before the password is typed.

- Area: cineris-cli
- Owner: the signed approvals step's second pull request, the maintainers

| Item | Who | When |
|---|---|---|
| **What is open.** A subcommand's core has no key until the person types the password, which it asks after showing what is approved (the signed approvals step). So `cineris permissions trust` judges a project's extensions with no key, finds none approved, and lists each again with "Approved before at this hash; your password confirms it again." A no keeps the pin; a yes signs the same. It is safe and noisy. Hiding what an unchecked pin names was not taken: it would let a pin another program wrote keep an extension from being offered. **Recommendation:** ask for the password before the listing, once the command's flow is changed to open the approvals first, and list against the key it makes. | The signed approvals step's second pull request, the maintainers | Before the first release |
