# Answered in the signed approvals step, on 2026-11-27

This work is not a numbered step of the build plan. It makes the harness's approvals the person's own where nothing confines the model's commands: [approvals-signed-with-a-key-that-needs-a-person](../../outstanding/approvals-signed-with-a-key-that-needs-a-person.md), and the part of [what-the-guards-of-the-harnesss-own-approvals-do-not-reach](../../outstanding/what-the-guards-of-the-harnesss-own-approvals-do-not-reach.md) that only a signature closes. It merges before step 29, whose `propose_extension` has the model write extensions for the person to approve. The work was done in parts, each with a record of its own: [parts/](parts/).

## The design, before building

Written before any code, then reviewed by an independent reviewer who had not seen the reasoning; the owner's and the maintainers' answers are in [The answers](#the-answers), and what the review changed is in [The review of the design](#the-review-of-the-design). Where the two disagree with the sections in between, they win.

### The threat note

**Who.** Code the model wrote or ran, with the user's rights, where nothing confines it: on Windows; on Linux without Landlock; in a workspace that holds Cineris's own files; and at `unrestricted` on every system. It can read and write every file the user can, run any program as the user (Cineris itself, a copy of it, or either inside a namespace of its own), and connect to the user's sockets.

**What it must not do.** Make an approval count that the person did not give, in any session, at any level, in either build: by writing a store, copying an entry from another project, putting back an entry from before, replacing what sessions check signatures against, or running Cineris's approving subcommands by any route. Today each of these is a write of a text file, or a command line the guards may miss. The review of step 28 found five lines past guard B: `cineris project`, which has no B; Windows' walk, which gives up at 64 interpreters; the socket, which answers another harness's command; a harness that is process 1; and a pid namespace of the command's own.

**What changes.** An approval counts only with a signature made by a key that only the person's password makes. The model's code can still run every approving subcommand, write every store, and answer every question over the socket. Without the password, nothing it writes counts. None of the review's five lines gets a signature by itself. Guards A, B and C stay as defence in depth: they keep the model from putting a real Cineris prompt in front of the person for something the model chose, which on Windows matters more than it seemed (the review's finding 1: a command shared the harness's console until this step).

### The trust anchor

A signature helps only if the model's code can neither sign without the person nor change what a session checks signatures against.

**The finding.** Whatever a session checks against while no person supplies a secret, a process with the user's rights can rewrite, unless it lives where only an administrator can write, or where the system asks the person before it is replaced. That includes a public key beside the stores, beside the binary, in the OS keyring or in the registry. The model makes a key of its own, writes its public half where the old one was, and signs what it likes. And an anchor that any program of the user's can make is a squat: the model can make it before the person does. So a session that checks without the person needs an anchor whose making and replacing need the person (an administrator's write, or the system's own consent prompt). Without such an anchor, a session needs the person's secret to check anything at all.

| Anchor | A process with the user's rights can | It cannot |
|---|---|---|
| **The password itself.** Argon2id of the password; the signing key is made from the result. Nothing stored is trusted by a session. | Read the stores and the key file's salt, and guess passwords offline at Argon2id's cost. Replace the key file: every session then fails to open the approvals, a denial and not a forgery. Read the keyboard where the system allows it (below). | Sign, or make a session accept a key of its own: a session checks with the key the typed password makes. |
| **A public key only an administrator can write.** `/etc/cineris/…` through `sudo`; under `%ProgramData%` (the known folder, not the variable) with an access list that gives the user read only, through UAC. | Read it. | Replace it, unless it can become an administrator without the person. **On Linux under WSL with Windows interop on (the default), any program of the user's becomes root through `wsl.exe -u root`, with no password** (to be confirmed on the reference machine). On a Windows administrator's account at UAC's default level, Microsoft does not treat UAC as a boundary, and public ways to elevate without a prompt exist. A `sudo` with `NOPASSWD` is the same. |
| **Windows Hello.** NCrypt's Passport key storage provider, through `windows-sys`; the public key asked of the system by the key's name. | Read the public key. Delete the key. Ask the system to make a new one, which shows the person Hello's prompt. | Sign, or make a key, without the person's consent at Hello's prompt. Windows only, and only where Hello is set up. |
| **A TPM key behind a PIN.** NCrypt's Platform Crypto Provider; `tpm2` on Linux. | Delete the key, and make another under the same name with a PIN of its own: making a key asks no one. | Sign with the person's key without its PIN. On most Linux systems the TPM is open only to the `tss` group. |
| **The OS keyrings.** Secret Service; the Windows credential store. | Read and write every item of the user's: neither keeps a secret from another program of the same user, nor an item from being replaced. | Nothing that matters here. |

**When the password is typed.** At each yes that is stored: the yes is the password. And, where the password is the anchor, once when a session opens with approvals to use. **Never once per login:** a key, or a check of one, kept for the rest of a login has to be kept somewhere a program of the user's can read or replace (the keyring, a file, an agent's socket). That is the anchor problem again.

**Recommended: the password is the anchor (P), on every system.** It needs no administrator and no device. It holds on both of the machines the measurements use, Linux under WSL included, where an administrator's anchor does not. Its cost is a prompt when a session opens with approvals to use, which the owner has not accepted yet. **Owner's call 1.**

| | P: the password is the anchor | A: an administrator holds the public key |
|---|---|---|
| Set-up | Choose a password | Choose a password, then `sudo` or UAC once to write the public key |
| A session with approvals to use | Asks for the password when it opens; Esc opens it without them | Asks nothing |
| A stored yes | The password | The password |
| `-p` | Asks at its terminal when it has one; with none, runs without approvals | Uses the approvals; a new one is refused |
| `serve` and `attach` | `attach` gives the server the password once, to open it; a server nobody opened uses no approvals | Uses the approvals; `attach` signs at the yes on its own side and sends only signatures |
| CI | Runs without approvals | Uses what the machine has stored (nothing, on a fresh runner) |
| A forgotten password | A reset drops every approval | A reset drops every approval, and needs `sudo` or UAC again |
| Linux under WSL, interop on | Holds against writes to the stores; interop gives the model's commands `powershell.exe` (a keyboard hook, `AttachConsole`) and `wsl.exe -u root` (the harness's memory) | Does not hold |
| A Windows administrator's account, UAC at its default | Holds against writes to the stores; the console and the keyboard are the residual below | Does not hold |
| A user with no administrator's rights | Holds | Cannot be set up |

Both use one format, one key made from the password, and one way of signing at the yes. They differ only in where a session's checking key comes from, so A can be added to P later as an option for the machines it holds on, sparing them the prompt when a session opens.

**Why not Hello or a TPM as the main path.** The person's sign-in must not depend on a device's own authentication, which not every machine offers. Hello is a possible extra on Windows (Owner's call 7). A TPM key adds nothing here: anything of the user's can replace it without consent, so it would need an anchor of its own.

### What signing cannot close

Each item below is something a process with the user's rights can still do. Each needs a deliberate attack on the person's own programs or input, rather than a write to the harness's stores.

- **A replaced or wrapped `cineris`.** It may be a binary in a folder the user can write (`~/.cargo/bin`, `~/.local/bin`, a per-user Windows install), a script of the same name earlier on `PATH`, or an alias in the shell's start-up file. It runs the next time the person types `cin`, with the person there. It can record what they type (`script` does it in one line), and then sign with the password. **Narrowed by:** an install to a folder only an administrator can write. That is step 34's to choose, and this step recommends it. The guards see only a session's commands, so they do not narrow this.
- **A keylogger, or a fake prompt.** On Windows any program of the user's can read the keyboard (a low-level keyboard hook) and attach to a console of the user's (`AttachConsole`) and read or write its input; so can any program on X11 with the keyboard. On Linux any program of the user's can open the person's terminal device and race the harness for its keys, or write a prompt there and read the answer: Wayland and the plain console narrow the keyboard, not the terminal device. A password typed where another program reads is that program's. **Narrowed by:** commands no longer share the harness's console on Windows (this step); while the password row is open the terminal device is opened exclusively on Linux (`TIOCEXCL`, which stops new opens, not ones already open); and the prompt refuses while a debugger is attached.
- **A debugger, or a read or write of the harness's memory.** On Linux a process of the user's can attach to the harness where Yama's `ptrace_scope` is 0. Ubuntu sets 1, which forbids it; other distributions differ. On Windows any process of the user's can open the harness and read or write its memory. With the password typed at each yes, the signing key is in memory only while a signature is made. Under P the checking key is in memory for the session, and a write to it would make that session accept forged entries. **Narrowed by:** the harness closing its own process (Owner's call 8).
- **An elevated process.** An administrator or root can do anything. Under A, the table above shows how close that is on WSL and on an administrator's Windows account.
- **A rollback of the stores.** No store can tell its latest state from an earlier signed one without storage the user cannot roll back. A TPM's monotonic counter would serve, but neither system opens one to a user's programs. A process that kept a copy of a store can put back an approval the person later took away. It can put back only an approval the person gave, and only for exactly what they saw then. **Narrowed by:**
  - every entry is bound to its project;
  - each unit (a project's trust, the user's pins) is signed whole with a revision, so an old pin comes back only with the whole unit it was signed in;
  - a reset makes a new key, after which nothing from before counts.

  **Owner's call 9.**
- **Guessing the password offline.** The stores and the key file's salt are readable, so a weak password can be guessed at Argon2id's cost: about 160 ms a guess on one core of the second machine at the proposed cost. A minimum length is Owner's call 6.
- **The person told what to type.** Text in the model's output can ask the person to type their password, or to approve something. The prompt always names what is being approved. It appears only at a session's start or after the person's own action.

### What is signed

Everything that approves or answers for the person, in the stores a session reads:

| Store | What each signature covers | Signed at |
|---|---|---|
| `pins.toml` | Every pin together: the revision, and each pin's kind, name, hash and registration | `cineris ext add`, the application's approval of an extension, step 30's approvals of the user's MCP servers and tools |
| `trust.toml` | Each project's trust, whole: its key, its revision, its root, whether git is trusted, and every pin's kind, name, hash and registration. A project's rules hash is one of these pins | The trust question, `cineris permissions trust`, approving what is new, step 30's approvals of a project's MCP servers |
| `approvals/<key>.toml` | Each stored approval: the project's key, its id, the rule | "Always allow in this project" |
| `projects/<key>/knowledge.toml` | Each entry accepted, and each decision on a shared entry for its hash. These make a learned command run unasked, a check run at a turn's end, or a fact reach the prompt. The review of step 28 wrote one through `cineris project add`, with no guard (its finding 1) | `cineris project add`, `accept`, `edit` and `promote`, and the application's accept |
| `extensions/registered.json` | What an approved extension registered: its tools, commands, hooks, prompt text, and which gates it holds. A session uses it in place of starting the engine, so a forged entry could add prompt text or drop a gate | To settle while building: bind each registration to its pin's signature where the registration is known at the yes (`ext add` evaluates before it asks), and otherwise evaluate again rather than use an unsigned registration |

**Superseded by the answer to call 4:** each unit of a store (a project's trust, the user's pins) is signed whole, with a revision, so taking a pin away is a signed change made at the person's yes; and what restricts counts unsigned (below, "The review of the design").

**Not covered by the stores: the user's own configuration.** Its permission rules, its level, its sandbox settings and its hooks grant with no approval, and the model's code can write the file. Signing the stores leaves that open. Closing it means pinning the configuration's permission-relevant parts whole, as a project's rules are pinned (dropping a deny is a widening), and asking after a hand edit, with the password, at the next start. **Answered:** the third pull request, merged before step 29.

### The formats

**The key file**, `approval-key.toml` in the state directory, is written owner-only and atomically. It is read strictly, with each cost capped so that a planted file cannot make a session take minutes or gigabytes:

```toml
version = 1
kdf = "argon2id"
memory_kib = 65536
passes = 3
lanes = 1
salt = "…"          # 16 random bytes
public_key = "…"    # what the password makes, to tell a mistyped password
made_at = "2026-11-27T10:15:02Z"
```

Under P a session never trusts `public_key`. It checks with the key the typed password makes, and uses `public_key` only to say "that is not the password". The key itself is made fresh each time and never stored: Argon2id of the password and the salt, then HKDF-SHA-256 with a label of its own, gives an Ed25519 seed.

**What is signed** is a statement's bytes: a domain label (`cineris approval v1`), the statement's kind (`user pin`, `project`, `project pin`, `stored approval`, `knowledge`), and each field with its length before it, as `Definition::of` hashes its parts.

**`pins.toml` version 2** has one `revision` and one `signature`, over every pin. **`trust.toml` version 3** gives each `[[project]]` a `revision` and a `signature`, over the project's key, revision, root, whether git is trusted, and every pin. **`approvals/<key>.toml` version 2** and **`knowledge.toml`'s next version** (the second pull request) sign each file's whole set the same way. The signature is Ed25519, 64 bytes written as 128 lowercase hexadecimal digits. (As built after the review: one signature for each unit, not each entry.)

What carries no signature keeps its present form and meaning: `refused`, `declined_rules`, `git = "declined"`, `root`, the times.

**The versions before are read as unsigned.** Nothing in them counts. Refusals and declines are still honoured, since each only holds something back. The next write drops the unsigned entries. They are not re-signed in bulk, because one the model wrote before the upgrade would be laundered. Each project and extension is asked about again as it is met. A build from before reads the new versions as it reads any newer version: it refuses the whole file, and so trusts nothing, which is safe.

**A forgotten password: `cineris approvals reset`.** It asks for a new password twice, makes a new salt, and drops every signed entry in every store, keeping the refusals and declines. Everything then asks again. Nothing keeps an approval across a reset, so there is no lock-out with no way out, and no reset that keeps what was approved. Under P, a reset the model runs with a password of its own changes nothing it can use. The person's password no longer opens the approvals, and the session says so and points to the reset. The model's own entries are checked with the person's key and fail.

`cineris approvals setup` makes the key, and so does the first yes that finds none. `cineris approvals status` says whether there is a key and what each store holds signed and unsigned. Setup and reset (and a later `password`, which re-signs under a new password) go into A's grammar as refused and into B's list. `status` only reads.

### The password prompt

**At a terminal** (the subcommands, `-p`):

- It reads the controlling terminal, not standard input: `/dev/tty`, and `CONIN$` on Windows.
- Echo is off.
- Whatever was typed before the prompt was drawn is thrown away (`tcflush`, `FlushConsoleInputBuffer`), so type-ahead is never taken as the password.
- A bracketed paste is taken without its markers.
- Ctrl-C cancels. The terminal's mode is put back on every way out, a signal and a panic included.

**In the application**, the prompt is a masked field in a dialog. While it is open, keys go to it alone: never to the input box, its history, the scrollback, the session log or a draft. Keys that were waiting when it opened are dropped, so a message being typed when it appears does not run on into it. The application enters raw mode before anything that can take time, so a password typed early is not echoed by the terminal's own line mode (the review of step 25's finding 1).

**Everywhere:**

- The password is held in a type that cannot be printed, logged or serialised, and is wiped when dropped (`zeroize`), as are the Argon2id output and the seed.
- No secret goes into the environment or a child's arguments, or across a protocol, apart from P's one unlock (below).
- Under call 8 the harness is not dumpable on Linux, so a crash leaves no core with a secret in it.

### Headless, the RPC server, CI

As recommended under P (A's differences are in the table above):

- **`-p`** asks at its terminal when it has one and the session has approvals to use. With none, it runs without them, and standard error says so once. A new approval in a headless run is refused, and the message names the command that asks for one (`cineris permissions trust`, `cineris ext add`). `--trust-project` stays what it is: trust for the run alone, stored nowhere, signing nothing.
- **`serve`** asks at its own terminal when it has one; with none it uses no approvals, and holds their gates. **No password ever crosses the socket** (answer C): a program that took over the socket's name would get it. An editor that uses `serve` with no terminal gets no project trust until an administrator's anchor (the third pull request) is set up where it holds.
- **A yes in `attach`** is signed in the `attach` process, which reads the stores itself and checks that what it signs is the stores as they are plus the change it shows, so that a server answering at a taken-over socket cannot slip in more. A raw client that answers `trust/request` (step 28's review, finding 3) has no signature to give: its yes is refused, and the question stays open.
- **CI** runs without approvals. What a CI run needs, it states on its own command line (`--allow`, `--permission-mode`). That is the CI file's statement, not an approval of the project's files. **No mode for "this machine has no person" is proposed**, since it would widen what runs (Owner's call 5).
- **A nested run** (a session's command running `cin -p`) has no person and no approvals. It is capped at `read-only` already.

### Speed and size

Measured on the second machine, release build:

| What | Cost | Against |
|---|---|---|
| Argon2id at the proposed cost (64 MiB, 3 passes, 1 lane) | 159 ms | Once at a session's opening under P, and at each stored yes: the person's time, after they press Enter. 2 passes take 119 ms; 46 MiB and 1 pass, 49 ms; 19 MiB and 2 passes, 33 ms. The cost is kept in the key file, so it can be raised later. |
| Checking one Ed25519 signature (`ring`) | 50 µs | Checked lazily: a project's trust when it is read, each pin when it is asked about, each stored approval when the engine is made. A session that opens with 5 to 30 to check would pay 0.25 to 1.5 ms of the headless budget's 15 ms, and nothing before the first frame. Under P a run with no terminal checks nothing. To measure on the reference machine and the runners with `perf_` tests in the first pull request. |
| Argon2id (`argon2` 0.6, no default features) and Ed25519 signing, beyond what `rustls`'s `ring` already links (X25519, Ed25519 checking, HKDF) | +44,088 bytes on Linux | A scratch program each way, built as the shipped binary is (`opt-level = "s"`, fat LTO, stripped), x86_64 with glibc. The musl and Windows builds will be measured in the first pull request (`cargo xtask size --breakdown`, `try … size --check --against-main`). Decision 0039 leaves 3,457,992 bytes on Linux and 4,989,952 on Windows before step 30. |

**This step's own code is estimated at 150 to 400 KB on Linux and 200 to 550 KB on Windows.** The estimate is taken high, because step 28's own code came in at about twice its guess.

**Dependencies:**

- `argon2` is new, and needs its row in [dependencies.md](../../dependencies.md).
- `ring` and `zeroize` become direct dependencies of `cineris-policy`. Both are in every shipped build already, through `rustls`.
- **Considered:** `ring`'s PBKDF2, which adds nothing to the binary but is cheap to guess with a GPU (Owner's call 10).
- **Hello, if chosen,** needs nothing new: NCrypt's Passport provider is reached through `windows-sys`.

### The interface for step 30

As agreed after the review (it replaces the sketch first sent). Step 30 stores its approvals in these stores, calling them at a person's yes:

```rust
// cineris_policy: reads take the session's key; with none, nothing counts.
PinStore::pins(&self, verifier: Option<&Verifier>) -> Result<UserPins, PinError>;
TrustStore::project(&self, key: &ProjectKey, verifier: Option<&Verifier>) -> Result<ProjectTrust, TrustError>;
// `standing`, `pinned_of` and `get` count only what is signed by that key;
// `held` gives every pin, signed or not: what restricts.

// Writes, removals included, are proposed, signed by the person, committed.
PinStore::propose(&self, changes: &[PinChange], verifier: &Verifier) -> Result<Option<PinProposal>, PinError>;
PinStore::commit(&self, proposal: &PinProposal, signature: &Signature, verifier: &Verifier) -> Result<UserPins, PinError>;
TrustStore::propose(&self, key: &ProjectKey, change: TrustChange, verifier: &Verifier) -> Result<Option<TrustProposal>, TrustError>;
TrustStore::commit(&self, proposal: &TrustProposal, signature: &Signature, verifier: &Verifier) -> Result<ProjectTrust, TrustError>;
// PinChange::{Pin(Approved), Unpin { kind, name }}
// TrustChange::{Trust { root, approved, git_shown, lift_refusal }, Approve { approved, git_shown }, Drop { pins }}
// Approved { definition, registration: Option<ContentHash> }: the registration hash for an extension, none for an MCP tool
// A proposal's `statement()` is what the person signs, and `summary()` what they read.
// `commit` writes nothing when the unit moved since the proposal (`Moved`: ask again)
// or the signature is not the key's over the proposal (`NotSigned`).

// cineris_policy::signed: a `Verifier` is made only by `PersonKey::verifier`.
```

In the protocol, a question whose yes is stored carries the proposal in structured form, and its answer one signature. The front end that asked the person rebuilds the statement itself and signs it; the core never holds a key.

### The pull requests

1. **The signed stores, the key and the password.**
   - `cineris-policy` gets `signed` (statements, Argon2id, Ed25519, the key file), and the five stores are signed and checked, with the old versions read as unsigned.
   - The password prompt goes into `cineris-term`, on both systems.
   - `cineris-cli` gets `approvals setup|reset|status`, and `ext add`, `permissions trust` and `project` sign at a terminal.
   - `cineris-core` holds a session's verifier.
   - The application asks to open the approvals when a session opens, and signs at the yes of each of its questions.
   - Guards A and B get the new subcommands, and the harness closes its own process (call 8).
   - Between this and the second pull request, `attach`'s yes holds for its session alone, as an answer that cannot be kept does today.
   - The tests are listed below.
2. **Headless and the RPC server:** `-p` at a terminal; `serve` and `attach` (the unlock, and signatures in answers); the protocol's additions and the regenerated schema.
3. **What the owner chooses of:** the user's configuration pinned (call 3); an administrator's anchor (call 1); Windows Hello (call 7).

Each pull request carries its own docs: `docs/security.md` sections 1 and 3 and the new mitigation's named test; [06](../../architecture/06-permissions-and-sandbox.md), [08](../../architecture/08-configuration.md), [11](../../architecture/11-rpc-protocol.md) and [12](../../architecture/12-extensions.md). The decision record is in the first. The two items are closed or narrowed by the last.

**The tests:**

- **The model's code writing each store:**
  - an unsigned entry;
  - one signed by another key;
  - the key file replaced together with entries signed under the new key;
  - an entry copied from another project;
  - a pin put back from before a re-trust;
  - an entry from before a reset;
  - the content changed under an old pin.

  Each is refused, in both builds.
- **The ways out:** a forgotten password and its reset; an old store asking again; an unreadable or unverifiable store failing closed.
- **The prompt, in pty tests with `send_key`:**
  - no echo;
  - type-ahead dropped;
  - a paste;
  - Ctrl-C putting the terminal back;
  - the password in no log, session log, scrollback or environment.
- **Windows:** on runners. Hello sits behind a trait with a fake, and its real path goes on a checklist for the reference machine's interactive session.
- **Breaks:** in `mutations/signed-approvals.breaks` from the start.

### The owner's calls

| # | Call | Recommended |
|---|---|---|
| 1 | Where a session's checking key comes from | **P: the password**, typed when a session opens with approvals to use. A, an administrator's anchor, as an option later where it holds (not on WSL with interop on, nor on an administrator's Windows account at UAC's default). |
| 2 | When the password is typed for a yes | **At each stored yes** (trust, an extension, "always allow in this project", accepting a learned entry). The alternative: once per session, after which the session signs until it ends. That is fewer prompts, but a key in memory for hours, and keys pressed into the person's terminal (which Windows and X11 allow any program of the user's) would sign. |
| 3 | The user's own configuration | **Pin its widening parts and ask after a hand edit**, in the third pull request. Until then, an item. |
| 4 | Taking an approval away, step 28's fixes' `drop_pins` included | **Unsigned:** each only makes something ask again. |
| 5 | A mode for a machine with no person | **None.** CI states what it needs on its command line. |
| 6 | A minimum length for the password | **8 characters,** with nothing else required. |
| 7 | Windows Hello, as an extra on Windows: wanted? Is Hello (a PIN at least) set up on the reference machine's Windows? | Hello in the third pull request if it is set up and wanted. **No TPM path.** |
| 8 | The harness closes its own process to the user's other programs: not dumpable on Linux (`PR_SET_DUMPABLE`, which also keeps a crash's core from holding a secret); on Windows an access list that denies the user reading, writing and threads, as the step 32 item recommends | **In this step,** since it narrows the residual that matters most to a key in memory. |
| 9 | A rollback of the stores (a withdrawn approval put back, exactly as it was given) | **Accept as the residual,** narrowed by whole-unit signatures and by reset. |
| 10 | Argon2id (+44 KB) or `ring`'s PBKDF2 (nothing added) | **Argon2id.** |

## The answers

| # | Call | Answered |
|---|---|---|
| 1 | Where a session's checking key comes from | **The password, now; an administrator's anchor later** (the owner, 2026-11-27): the first two pull requests are password-anchored; the third adds a public key only an administrator can write, as an option offered at set-up where it holds (native Linux with `sudo`), and refused or warned of on WSL with interop on and on an administrator's account at UAC's default, so that those sessions skip the prompt. |
| 2 | When the password is typed for a yes | **At each stored yes,** with no signing key held in the session (the owner). |
| 3 | The user's own configuration | **Pinned whole** in the third pull request, before step 29: a change that widens rules (a deny dropped included), the level or the sandbox, or adds a hook, takes effect only after the password; a narrowing applies at once (the owner, and the maintainers on its timing). |
| 4 | Taking an approval away | **Signed, as a change of the whole unit** (the maintainers, with the reviewer): a pin can hold a gate, so its removal is a widening. See below. |
| 5 | A mode for a machine with no person | **None** (the maintainers). |
| 6 | The password's minimum length | **12 characters,** nothing else required (the maintainers, after the reviewer's finding that 8 is weak against offline guessing). |
| 7 | Windows Hello, a TPM | **Neither** (the owner). |
| 8 | The harness closes its own process | **In this step** (the maintainers): not dumpable on Linux; on Windows an access list, on the process and through the token's default list on its threads, that leaves the user only `SYNCHRONIZE`, `PROCESS_QUERY_LIMITED_INFORMATION` and `PROCESS_TERMINATE`. |
| 9 | A rollback of the stores | **The stated residual** (the maintainers), with the review's list of ways below. |
| 10 | Argon2id or PBKDF2 | **Argon2id** (the maintainers). |
| A | Commands on the harness's console (Windows) | **Started without it, in this step** (the owner), measured against `main` on a runner. |
| B | The managed configuration | **Its folder read through the system's known folder, and a managed file the user can write refused,** on both systems, in this step (the maintainers). |
| C | A password over the socket | **Never** (the maintainers): `serve` asks at its own terminal or runs without approvals; `attach` signs on its own side. |
| E | When the administrator's anchor comes | **The third pull request, straight after the second** (the maintainers). |

**The pull requests** (re-cut after the review, agreed by the maintainers; all three merge before step 29):

1. The signed module and the key file; `pins.toml` and `trust.toml` signed whole, with the restriction side; the CLI's approving subcommands signing at a terminal (`ext add` and `remove`, `permissions trust`, `approvals setup|reset|status`); the application asking for the password when a session opens and signing at its trust question and its approval of an extension; guards A and B for the new subcommands; the harness's process closed, commands off its console, and the managed file the administrator's. **Until the second, the stored approvals and the accepted entries of what was learned are as unsigned as before:** that pull request's record says so.
2. Stored approvals, and accepted entries of what was learned (signed by the core, since `cineris-knowledge` cannot see the policy crate); the registration cache bound to the hash in its pin; headless at a terminal, `serve` and `attach`; the protocol's additions.
3. The user's configuration pinned whole, and the administrator's anchor where it holds.

## The review of the design

An independent review read the note against the code before anything was built (6 high, 6 medium and 7 low findings). Each, and what was done:

| Finding | What it found | Done |
|---|---|---|
| 1, high | Every command shared the harness's console on Windows (`CREATE_NEW_PROCESS_GROUP \| CREATE_SUSPENDED` only): it could read the password as it was typed, press keys into any question, or draw a real prompt for the model's choice | Commands start without the harness's console (answer A); the `AttachConsole` residual is stated and on the reference machine's checklist |
| 2, high | WSL was listed as holding, and the keyboard residual as Windows and X11 only | The rows say what holds against what; interop and the terminal device are in the residual |
| 3, high | The user's and the managed configuration grant unsigned; the managed folder came from a variable | The user's configuration pinned whole (answer 3); the managed folder from the known folder, and a managed file the user can write refused (answer B) |
| 4, high | Unsigned removals widen where a pin holds a gate, and a session with no key dropped every extension gate | Whole units signed with a revision (answer 4); what restricts counts unsigned, so a session opened without the person's approvals, and a headless run, keep every gate |
| 5, high | An unlock over the socket would send the password to whatever answers there | No password crosses the socket (answer C) |
| 6, medium | A raw client can still answer questions that are not stored | From the first pull request an unsigned yes trusts nothing, not even for its session; refusing another harness's command at the socket is step 28's fixes' |
| 7, medium | Facts the harness observed reach the prompt unsigned; flags outside the hash | Observed facts are said to be prompt text as a repository's own files are; what is signed of an accepted entry includes its status and expiry, and a disable takes its signature away (the second pull request) |
| 8, medium | The registration cache was left open | The hash of what an extension registered is in its pin's signed statement; an extension is evaluated before it is asked about; a registration not known, and a build without the engine, hold both gates and add no prompt text |
| 9, medium | Rollback understated; one pin could be put back alone | Whole units: a pin comes back only with the whole unit it was signed in. Rollback needs no copy made by the model: `knowledge.toml.good`, a hard link to a store file, Windows's Previous Versions and File History, file-system snapshots (btrfs, ZFS). `approvals password` is dropped: re-signing would sign a rolled-back entry; a reset makes a new key |
| 10, medium | Nothing kept a session from checking with a key read from a file | `Verifier` can be made only from the key a typed password makes: no constructor from bytes, text or a file |
| 11, medium | The hardening's own interactions: nested runs, `hidepid`, a tracer attached first, thread access lists, the creator's handle | Each handled in the hardening part or stated; the prompt refuses while a debugger is attached |
| 12, medium | Under the password anchor, one approved extension makes every session ask, and a prompt at every start makes a fake one look normal | Accepted as the cost of the password anchor until the administrator's anchor (answer E) |
| Low | The password typed into the message box; wiping with `ring`; the cost's floor; 8 characters; status wording; reset blocked; the note and the draft disagreeing | The message box and wiping: residual, under the memory protection of answer 8; set-up and reset always write the standard cost; 12 characters; status says "carries a signature, not checked"; reset moves aside what blocks it; the note is corrected here (signatures are hexadecimal; whole units) |
