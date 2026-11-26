The secret filter masks an AWS access key id (`AKIA…`) but not the secret access key printed beside it, and `/export` and the first run's import rely on the filter's formats.

- Area: cineris-base
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **Half of an AWS key pair is masked.** `cineris_base::SecretFilter`'s formats (`crates/cineris-base/src/redact.rs`) include AWS's access key ids (`AKIA` or `ASIA` and 16 characters); the secret access key has no prefix (40 characters of base64), so a pair printed together (`aws configure list`, a `credentials` file shown, an environment dump) is masked in its id only. [security.md](../security.md), section 4, says the filter is narrow on purpose and does not catch a secret with no recognisable shape. Older than step 25, but step 25's `/export`, whose client pass masks by format only, and the first run's import, which refuses a server by the filter's formats, rely on it. Noted by the review of step 25; not tested. **Recommendation:** mask a 40-character base64 value that follows one of AWS's names for the secret (`aws_secret_access_key`, `AWS_SECRET_ACCESS_KEY`, `SecretAccessKey`) in the same text, and say so in security.md's row. | Step 32 (hardening) | Before the first release |
