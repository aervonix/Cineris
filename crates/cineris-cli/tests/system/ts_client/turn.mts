// A client of `cineris serve --stdio` in TypeScript that knows the protocol
// only from the generated declarations, and runs one turn with it
// (docs/steps/27/answered.md, point 7; docs/architecture/11-rpc-protocol.md).
//
//   node turn.mts CINERIS WORKSPACE PROMPT [CHOICE]
//
// It starts the binary CINERIS as a server on its standard I/O, says
// `initialize` as a `user` client that answers approvals, opens a session on
// WORKSPACE, sends PROMPT, and reads the session's events until the turn has
// completed and the session is idle. Each approval it is asked is answered
// CHOICE (`deny` when not given, or when CHOICE is not on offer). Then it
// prints one line of JSON: the turn, its outcome, the assistant's text and
// the choices it answered. It sends `shutdown` and exits 0 once the server
// has exited 0. A JSON-RPC error, a message it cannot read, a field the
// declarations promise that is not there, or an end before the turn's, exits
// 1 with the reason on standard error.
//
// Node 22.18 or later runs it as it is, stripping the types: so it uses only
// syntax that can be stripped, and its imports of types are `import type`,
// which leave nothing behind. Node checks no types; `tsc` does, with
// tsconfig.json beside this file (docs/testing-and-ci.md, "The TypeScript
// client"). Every field it reads is read through `need`, whose key the
// compiler checks against the declarations and whose value is checked when
// the message comes: a field renamed or removed in the protocol fails the
// type check and the run alike.

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

import type {
  ApprovalChoice,
  ClientMethods,
  JsonRpcMessage,
  JsonRpcResponse,
  ProtocolVersion,
  RequestId,
  ServerMethods,
  ServerNotifications,
  SessionEvent,
  StopReason,
  TurnId,
  TurnOutcome,
} from "../../../../../docs/protocol/ts/cineris-protocol.d.ts";

/** The protocol version this client speaks: the declarations' own. */
const VERSION: ProtocolVersion = 1;

/** What the run prints, on one line. */
interface Summary {
  turn_id: TurnId;
  outcome: TurnOutcome;
  stop_reason: StopReason | null;
  /** The text of the turn's assistant messages, in order. */
  text: string;
  /** The answer given to each approval, in order. */
  approvals: ApprovalChoice[];
}

/** A failure the client reports and exits 1 for. */
class Failure extends Error {}

/**
 * The field `key` of `value`, which the declarations say it has. The key is
 * checked by the compiler, and the value when the message comes: absent (or
 * `null`), the run fails and says which.
 */
function need<T extends object, K extends keyof T & string>(
  value: T,
  key: K,
  what: string,
): T[K] & {} {
  const found = value[key];
  if (found === undefined || found === null) {
    throw new Failure(`${what} has no \`${key}\`: ${JSON.stringify(value)}`);
  }
  return found;
}

/** An object, as a message's parameters must be. */
function object(value: unknown, what: string): object {
  if (typeof value !== "object" || value === null) {
    throw new Failure(`${what} is not an object: ${JSON.stringify(value)}`);
  }
  return value;
}

type Method = keyof ClientMethods;
type Params<M extends Method> = ClientMethods[M]["params"];
type Result<M extends Method> = ClientMethods[M]["result"];

/** A request sent and not answered yet. */
interface Waiting {
  method: Method;
  answered: (result: unknown) => void;
  failed: (error: Error) => void;
}

/** A completed turn, with where it was in the session's events. */
interface Completed {
  seq: number;
  outcome: TurnOutcome;
  stop_reason: StopReason | null;
}

async function main(): Promise<void> {
  const [cineris, workspace, prompt, given] = process.argv.slice(2);
  if (cineris === undefined || workspace === undefined || prompt === undefined) {
    throw new Failure("usage: node turn.mts CINERIS WORKSPACE PROMPT [CHOICE]");
  }
  const choice: ApprovalChoice = given ?? "deny";

  const server = spawn(cineris, ["serve", "--stdio"], {
    stdio: ["pipe", "pipe", "inherit"],
  });

  // Everything that goes wrong while the client waits ends the wait.
  let broken: (error: Error) => void = () => {};
  const failure = new Promise<never>((_, reject) => {
    broken = reject;
  });
  failure.catch(() => {});
  const wait = <T,>(promise: Promise<T>): Promise<T> => Promise.race([promise, failure]);

  let ending = false;
  const exited = new Promise<number | null>((resolve) => {
    server.on("exit", (code) => resolve(code));
  });
  server.on("error", (error) => broken(new Failure(`${cineris} did not start: ${error.message}`)));
  server.stdin.on("error", (error) => broken(new Failure(`writing to the server: ${error.message}`)));

  function send(message: JsonRpcMessage): void {
    server.stdin.write(`${JSON.stringify(message)}\n`);
  }

  let next = 0;
  const waiting = new Map<RequestId, Waiting>();
  function call<M extends Method>(method: M, params: Params<M>): Promise<Result<M>> {
    next += 1;
    const id = next;
    const answer = new Promise<Result<M>>((resolve, reject) => {
      waiting.set(id, {
        method,
        // The result is the declarations' type for the method; what of it
        // is read is read through `need`.
        answered: (result) => resolve(result as Result<M>),
        failed: reject,
      });
    });
    send({ jsonrpc: "2.0", id, method, params });
    return wait(answer);
  }

  function onResponse(response: JsonRpcResponse): void {
    const id = response.id;
    const asked = id === null ? undefined : waiting.get(id);
    if (id === null || asked === undefined) {
      throw new Failure(`an answer to no request: ${JSON.stringify(response)}`);
    }
    waiting.delete(id);
    if (response.error !== undefined) {
      const error = response.error;
      const data = typeof error.data === "object" && error.data !== null ? error.data : {};
      const code = "code" in data ? ` (${String(data.code)})` : "";
      asked.failed(new Failure(`${asked.method} failed: ${error.message}${code}`));
      return;
    }
    if (!("result" in response)) {
      throw new Failure(`${asked.method}: a response with neither result nor error`);
    }
    asked.answered(response.result);
  }

  const approvals: ApprovalChoice[] = [];
  function onRequest(id: RequestId, method: string, params: unknown): void {
    if (method !== "approval/request") {
      // The client said it answers approvals and nothing else.
      send({
        jsonrpc: "2.0",
        id,
        error: { code: -32601, message: `this client does not answer \`${method}\`` },
      });
      return;
    }
    const request = object(params, method) as ServerMethods["approval/request"]["params"];
    const offered = need(request, "choices", method);
    need(request, "call_id", method);
    const answer: ServerMethods["approval/request"]["result"] = offered.includes(choice)
      ? { choice }
      : { choice: "deny", note: `the TypeScript client was told to answer ${choice}` };
    approvals.push(answer.choice);
    send({ jsonrpc: "2.0", id, result: answer });
  }

  // The session's events, as far as the summary needs them.
  let session: string | undefined;
  const texts: { turn: TurnId | undefined; text: string }[] = [];
  const completed = new Map<TurnId, Completed>();
  let idle = -1;
  let settledCheck: () => void = () => {};

  function onEvent(event: SessionEvent): void {
    const seq = need(event, "seq", "an event");
    need(event, "type", "an event");
    if (need(event, "session_id", "an event") !== session) {
      return;
    }
    switch (event.type) {
      case "item.completed": {
        const item = need(event, "item", event.type);
        if (item.kind === "assistant_message") {
          for (const part of need(item, "parts", "an assistant message")) {
            if (part.kind === "text") {
              texts.push({ turn: event.turn_id, text: need(part, "text", "a text part") });
            }
          }
        }
        break;
      }
      case "turn.completed":
        completed.set(need(event, "turn_id", event.type), {
          seq,
          outcome: need(event, "outcome", event.type),
          stop_reason: event.stop_reason ?? null,
        });
        break;
      case "session.idle":
        idle = seq;
        break;
      default:
        // Every other event, and any type this version does not know.
        break;
    }
    settledCheck();
  }

  function onLine(line: string): void {
    if (line.trim() === "") {
      return;
    }
    let read: unknown;
    try {
      read = JSON.parse(line);
    } catch {
      throw new Failure(`not a JSON-RPC message: ${line}`);
    }
    const message = object(read, "a message") as JsonRpcMessage;
    if ("method" in message) {
      if ("id" in message) {
        onRequest(message.id, message.method, message.params);
      } else if (message.method === "session/event") {
        onEvent(object(message.params, message.method) as ServerNotifications["session/event"]);
      }
      return;
    }
    onResponse(message);
  }

  const lines = createInterface({ input: server.stdout, crlfDelay: Infinity });
  lines.on("line", (line) => {
    try {
      onLine(line);
    } catch (error) {
      broken(error instanceof Error ? error : new Failure(String(error)));
    }
  });
  lines.on("close", () => {
    if (!ending || waiting.size > 0) {
      broken(new Failure("the server's output ended before the client was done"));
    }
  });

  try {
    const hello = await call("initialize", {
      protocol_version: VERSION,
      client: { name: "cineris-ts-client", version: "0" },
      role: "user",
      capabilities: { approvals: true },
    });
    const spoken = need(hello, "protocol_version", "initialize's result");
    if (spoken !== VERSION) {
      throw new Failure(`the server speaks protocol version ${spoken}, not ${VERSION}`);
    }

    const opened = await call("session/new", { workspace });
    session = need(need(opened, "session", "session/new's result"), "session_id", "the session");

    const started = await call("turn/start", {
      session_id: session,
      input: [{ kind: "text", text: prompt }],
    });
    const turn = need(started, "turn_id", "turn/start's result");

    // The turn is over once it has completed and the session is idle after
    // it: its events may come before `turn/start`'s result, so they are kept
    // by the turn they name.
    const over = await wait(
      new Promise<Completed>((resolve) => {
        settledCheck = () => {
          const done = completed.get(turn);
          if (done !== undefined && idle > done.seq) {
            resolve(done);
          }
        };
        settledCheck();
      }),
    );

    const summary: Summary = {
      turn_id: turn,
      outcome: over.outcome,
      stop_reason: over.stop_reason,
      text: texts
        .filter((entry) => entry.turn === turn)
        .map((entry) => entry.text)
        .join(""),
      approvals,
    };
    process.stdout.write(`${JSON.stringify(summary)}\n`);

    ending = true;
    await call("shutdown", {});
    server.stdin.end();
    const code = await exited;
    if (code !== 0) {
      throw new Failure(`the server exited with ${code}, not 0`);
    }
  } catch (error) {
    server.kill();
    throw error;
  }
}

main().catch((error: unknown) => {
  process.stderr.write(`turn.mts: ${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
});
