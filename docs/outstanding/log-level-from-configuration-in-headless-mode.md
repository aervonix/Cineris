A headless run starts the log before configuration is read, so `log.level` is ignored there (the interactive half is done).

- Area: cortex-core
- Owner: step 32

| Item | Who | When |
|---|---|---|
| **`log.level` from configuration, in a headless run.** The binary starts the log before the core reads configuration, so its filter comes from `--log-level` and `CORTEX_LOG` only ([10-headless.md](../architecture/10-headless.md), "As built in step 13b"). Applying the configured level means setting the filter again once the core has started (`LogHandle::set_filter`), which needs the core to say what the user's configuration holds without a session: a small addition to `cortex-core`. **Step 23 built the interactive half:** the binary reads `log.level` from the user's configuration before the log starts, and hands the same load to `Core::start` (`CoreOptions::loaded`), with `--log-level` and `CORTEX_LOG` above it. **The headless half is left:** a headless run still starts its log from `--log-level` and `CORTEX_LOG` alone. The same load before the log, handed to the core, closes it; the start-up trace's allow-list already has the user's configuration on the path. | Step 32 (the hardening pass), or the next step that changes the headless start; the interactive half done in step 23 | Before the first release |
