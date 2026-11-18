NTFS updates a directory entry's stamp lazily while a writer holds a file open, so the index may read it every pass or keep an old outline.

- Area: cineris-core
- Owner: a later index step, or step 33

| Item | Who | When |
|---|---|---|
| **A file held open for writing on NTFS, and its stamp in the index.** A pass lists a file's size and modification time from its directory entry (on Windows, what `FindNextFile` gives), and records the stamp it read from the open file. NTFS updates a directory entry's size and times lazily while a writer holds the file open, so the two can differ for as long as the writer does: the file is read again by every pass (its record never holds), or, if the entry is stale in the other direction, keeps an old outline until the writer closes it (a note of the independent review of step 18, by reading; not seen in a test). Its cost is bounded: the map shows a file whole or not, the `outline` tool and `read` read the file as it is now, and a file read by every pass is one read a pass. **What a fix would take:** record the listed stamp beside the opened one, and treat a record as holding when the listed stamp matches the listed stamp it was made from; or, on Windows, stamp a file from its handle in the listing too (a `GetFileInformationByHandle` per file, which costs an open per file on every pass, about what the listing costs now); and a Windows test that holds a file open for writing across two passes. | A later step on the index, or step 33 | When a Windows user reports an outline that stays old, or before the first release |
