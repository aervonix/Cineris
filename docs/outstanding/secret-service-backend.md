The Linux Secret Service credential backend needs a non-blocking CI job against a real keyring daemon.

- Area: cortex-config
- Owner: a step the owner places

| Item | Who | When |
|---|---|---|
| **The Secret Service backend of the credential store,** for the Linux `keychain` feature ([decision 0018](../decisions/0018-credential-storage.md)). Step 9 built the store selection, the feature name and the error a build without the backend gives. The backend needs a non-blocking CI job that runs it against a real keyring daemon, because the blocking jobs cannot. | A step the owner places | Before the first release |
