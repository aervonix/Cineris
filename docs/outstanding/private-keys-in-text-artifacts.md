Text artifacts are redacted line by line, so a PEM private key is masked on one line only.

- Area: cortex-session
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **A private key printed into a text artifact is masked on its first line only.** The session store redacts a text artifact one line at a time, and only the first line of a key in PEM form says what it is. In a log entry, where the filter sees the whole text, the key is masked whole ([security.md](../security.md), section 4). Closing it needs the redaction hook to keep state from one line of an artifact to the next. | Step 32 (the hardening pass) | Before the first release |
