# Answered in the signed approvals step, on 2026-11-27

This work is not a numbered step of the build plan. It makes the harness's approvals the person's own where nothing confines the model's commands: [approvals-signed-with-a-key-that-needs-a-person](../../outstanding/approvals-signed-with-a-key-that-needs-a-person.md), and the part of [what-the-guards-of-the-harnesss-own-approvals-do-not-reach](../../outstanding/what-the-guards-of-the-harnesss-own-approvals-do-not-reach.md) that only a signature closes. It merges before step 29, whose `propose_extension` has the model write extensions for the person to approve.

## The design, before building

Written before any code. The calls marked **Owner's call** are the owner's; nothing below is built until they are answered.

### The threat note

**Who.** Code the model wrote or ran, with the user's rights, where nothing confines it: on Windows; on Linux without Landlock; in a workspace that holds Cineris's own files; and at `unrestricted` on every system. It can read and write every file the user can, run any program as the user (Cineris itself, a copy of it, or either inside a namespace of its own), and connect to the user's sockets.

**What it must not do.** Make an approval count that the person did not give, in any session, at any level, in either build: by writing a store, copying an entry from another project, putting back an entry from before, replacing what sessions check signatures against, or running Cineris's approving subcommands by any route. Today each of these is a write of a text file, or a command line the guards may miss. The review of step 28 found five lines past guard B: `cineris project`, which has no B; Windows' walk, which gives up at 64 interpreters; the socket, which answers another harness's command; a harness that is process 1; and a pid namespace of the command's own.

**What changes.** An approval counts only with a signature made by a key that only the person's password makes. The model's code can still run every approving subcommand, write every store, and answer every question over the socket. Without the password, nothing it writes counts. None of the review's five lines gets a signature, so they stop mattering for approvals. Guards A, B and C stay as defence in depth: they keep the model from putting a real Cineris prompt in front of the person for something the model chose.

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
| Linux under WSL, interop on | Holds | Does not hold |
| A Windows administrator's account, UAC at its default | Holds | Does not hold |
| A user with no administrator's rights | Holds | Cannot be set up |

Both use one format, one key made from the password, and one way of signing at the yes. They differ only in where a session's checking key comes from, so A can be added to P later as an option for the machines it holds on, sparing them the prompt when a session opens.

**Why not Hello or a TPM as the main path.** The person's sign-in must not depend on a device's own authentication, which not every machine offers. Hello is a possible extra on Windows (Owner's call 7). A TPM key adds nothing here: anything of the user's can replace it without consent, so it would need an anchor of its own.

### What signing cannot close

Each item below is something a process with the user's rights can still do. Each needs a deliberate attack on the person's own programs or input, rather than a write to the harness's stores.

- **A replaced or wrapped `cineris`.** It may be a binary in a folder the user can write (`~/.cargo/bin`, `~/.local/bin`, a per-user Windows install), a script of the same name earlier on `PATH`, or an alias in the shell's start-up file. It runs the next time the person types `cin`, with the person there. It can record what they type (`script` does it in one line), and then sign with the password. **Narrowed by:** an install to a folder only an administrator can write. That is step 34's to choose, and this step recommends it. The guards see only a session's commands, so they do not narrow this.
- **A keylogger, or a fake prompt.** On Windows, any program of the user's can read the keyboard (a low-level keyboard hook); so can any program on X11. Any program can also write a prompt to the person's terminal device and read the answer. A password typed there is the attacker's. This does not hold on Wayland or a plain console, where one program of the user's cannot read another's keys. Not narrowed. Where Hello is chosen, its prompt is drawn by the system, not the terminal.
- **A debugger, or a read or write of the harness's memory.** On Linux a process of the user's can attach to the harness where Yama's `ptrace_scope` is 0. Ubuntu sets 1, which forbids it; other distributions differ. On Windows any process of the user's can open the harness and read or write its memory. With the password typed at each yes, the signing key is in memory only while a signature is made. Under P the checking key is in memory for the session, and a write to it would make that session accept forged entries. **Narrowed by:** the harness closing its own process (Owner's call 8).
- **An elevated process.** An administrator or root can do anything. Under A, the table above shows how close that is on WSL and on an administrator's Windows account.
- **A rollback of the stores.** No store can tell its latest state from an earlier signed one without storage the user cannot roll back. A TPM's monotonic counter would serve, but neither system opens one to a user's programs. A process that kept a copy of a store can put back an approval the person later took away. It can put back only an approval the person gave, and only for exactly what they saw then. **Narrowed by:**
  - every entry is bound to its project;
  - a project trusted again gets a new generation, so an old pin cannot be mixed into a new trust;
  - a reset makes a new key, after which nothing from before counts.

  **Owner's call 9.**
- **Guessing the password offline.** The stores and the key file's salt are readable, so a weak password can be guessed at Argon2id's cost: about 160 ms a guess on one core of the second machine at the proposed cost. A minimum length is Owner's call 6.
- **The person told what to type.** Text in the model's output can ask the person to type their password, or to approve something. The prompt always names what is being approved. It appears only at a session's start or after the person's own action.

### What is signed

Everything that approves or answers for the person, in the stores a session reads:

| Store | What each signature covers | Signed at |
|---|---|---|
| `pins.toml` | Each pin: kind, name, hash | `cineris ext add`, the application's approval of an extension, step 30's approvals of the user's MCP servers and tools |
| `trust.toml` | Each project's trust: its key, its generation (16 random bytes, made when it was trusted), and whether git is trusted. Each pin: the project's key and generation, kind, name, hash. A project's rules hash is one of these pins | The trust question, `cineris permissions trust`, approving what is new, step 30's approvals of a project's MCP servers |
| `approvals/<key>.toml` | Each stored approval: the project's key, its id, the rule | "Always allow in this project" |
| `projects/<key>/knowledge.toml` | Each entry accepted, and each decision on a shared entry for its hash. These make a learned command run unasked, a check run at a turn's end, or a fact reach the prompt. The review of step 28 wrote one through `cineris project add`, with no guard (its finding 1) | `cineris project add`, `accept`, `edit` and `promote`, and the application's accept |
| `extensions/registered.json` | What an approved extension registered: its tools, commands, hooks, prompt text, and which gates it holds. A session uses it in place of starting the engine, so a forged entry could add prompt text or drop a gate | To settle while building: bind each registration to its pin's signature where the registration is known at the yes (`ext add` evaluates before it asks), and otherwise evaluate again rather than use an unsigned registration |

**Taking an approval away is not signed.** This covers `unpin`, `revoke`, `forget`, refusing a project, keeping its trust without what is new, and step 28's fixes' `drop_pins`. Each can only make something ask again, which is safe; a program of the user's can delete a store anyway. The doc of `drop_pins` calls a drop an approval change, and it is one, but in the direction that needs no person (Owner's call 4). An unsigned entry counts for nothing, not even as `Changed`: its hash is the writer's word. All a session says of it is that it asks again.

**Not covered by the stores: the user's own configuration.** Its permission rules, its level, its sandbox settings and its hooks grant with no approval, and the model's code can write the file. Signing the stores leaves that open. Closing it means pinning the configuration's widening parts as a project's are pinned, and asking after a hand edit, with the password, at the next start. The user's MCP servers are pinned already, by step 30. **Owner's call 3.**

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

**`pins.toml` version 2** gives each `[[pin]]` a `signature`. **`trust.toml` version 3** gives each `[[project]]` a `generation` and a `signature`, and each `[[project.pinned]]` a `signature`. **`approvals/<key>.toml` version 2** and **`knowledge.toml`'s next version** give each signed entry a `signature`. The signature is Ed25519, 64 bytes in base64.

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
- **`serve`** uses no approvals until a `user` client opens them with a new method, `approvals/unlock`. `attach` asks the person when it starts, as the application does. The method's password is a type no log records, and it is wiped after the key is made.
- **A yes in `attach`** is signed in the `attach` process. The server checks the signatures with the key the unlock made, then stores them, so the password does not cross the socket again. A program that stands a server of its own at the socket's path can show `attach` anything; that is the fake prompt above. A raw client that answers `trust/request` (the review's finding 3) has no signature to give: its yes is refused, and the question stays open.
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

Step 30 stores its approvals in these stores and calls `PinStore::pin`/`unpin`, `TrustStore::approve`, `ProjectTrust::standing`/`pinned_of` and `drop_pins`. After this step:

```rust
// cineris_policy::signed
pub struct Verifier;      // the person's public key: what a session checks with
pub struct Generation;    // a trusted project's 16 random bytes
pub enum Statement {      // one approval, as signed
    UserPin { kind: TrustResourceKind, name: String, hash: ContentHash },
    ProjectPin { project: ProjectKey, generation: Generation, kind: TrustResourceKind, name: String, hash: ContentHash },
    // Project, StoredApproval, Knowledge …
}
impl Statement { pub fn bytes(&self) -> Vec<u8>; }
pub struct Signatures;    // what a person's yes carries: a signature for each statement

// Reads take the session's verifier; with none, nothing is approved.
// `UserPins::standing`, `ProjectTrust::standing` and `pinned_of` keep their signatures,
// and count only what verified.
PinStore::pins(&self, verifier: Option<&Verifier>) -> Result<UserPins, PinError>;
TrustStore::project(&self, key: &ProjectKey, verifier: Option<&Verifier>) -> Result<ProjectTrust, TrustError>;

// A write that approves takes the person's signatures for exactly what it writes. The
// store makes each statement itself, finds its signature, checks it, and writes nothing
// if any is missing or wrong.
PinStore::pin(&self, definition: &Definition, signed: &Signatures, verifier: &Verifier) -> Result<UserPins, PinError>;
TrustStore::approve(&self, key: &ProjectKey, definitions: &[Definition], git_shown: bool,
                    signed: &Signatures, verifier: &Verifier) -> Result<ProjectTrust, TrustError>;
ProjectTrust::generation(&self) -> Option<&Generation>;  // what a project pin's statement names

// Taking away is unchanged: no signature.
PinStore::unpin(&self, kind: &TrustResourceKind, name: &str) -> Result<bool, PinError>;
TrustStore::drop_pins(/* as step 28's fixes give it */);
```

**In the protocol**, a question whose yes is stored carries the statements its yes signs. Its answer carries `signatures`, one for each statement, in order. The front end builds each statement's bytes with the same library, checks that every statement is something it shows, asks the password, and signs. Step 30's approval questions follow the same pattern. The application, in process, does exactly what `attach` does. The core never sees the password at a yes.

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
| 9 | A rollback of the stores (a withdrawn approval put back, exactly as it was given) | **Accept as the residual,** narrowed by generations and by reset. |
| 10 | Argon2id (+44 KB) or `ring`'s PBKDF2 (nothing added) | **Argon2id.** |
