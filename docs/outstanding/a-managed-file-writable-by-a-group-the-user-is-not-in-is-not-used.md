On Linux a managed configuration file, or a folder above it, that its group may write is not used even when the user is not in that group, so an administrator's group-writable `/etc/cineris` lifts the administrator's own locks with only a warning.

- Area: cineris-config
- Owner: the signed approvals step's third pull request, the maintainers

| Item | Who | When |
|---|---|---|
| **What is open.** Since the signed approvals step a managed file the user can change is not used ([08-configuration.md](../architecture/08-configuration.md), "The managed file is an administrator's"). On Linux the check (`managed::unix_refusal`) refuses a file or folder on the way that its group may write, whatever the group, and one owned by anyone but root. The OS layer's review of the step (its finding F5) found that this refuses an administrator's file the user cannot change: `/etc/cineris` at `2775 root:cfgadmins`, or a file a service account owns, is then not used, and the administrator's locks lift for every user, with a warning. It fails open on policy for a file the model's code cannot write; Windows judges with the process's own token and does not. **Recommendation:** refuse group write only when the file's group is the process's effective group or one of its supplementary groups (read from `/proc/self/status`, so that no `unsafe` is needed), and keep the owner rule as stated. | The signed approvals step's third pull request, the maintainers | Before the first release |
