// The native messaging client (docs/BROWSER-INTEGRATION.md, "Native
// messaging"): one `sendNativeMessage` per message (an MV3 service worker must
// not depend on a long-lived port), a timeout longer than the host's own
// waits, and replies checked before anything acts on them. Every failure comes
// back as a value, never a throw, so a caller cannot forget the fallback.
import {
  DEV_HOST_NAME,
  HOST_NAME,
  OLDEST_VERSION,
  VERSION,
  type CaptureRules,
  type Envelope,
  type Message,
  type ReplyEnvelope,
} from "@boltget/native-messaging";

/**
 * How long to wait for a reply. The host waits up to 8 s for the app to start
 * and 6 s for its answer, so this must exceed 15 s: a late `accepted` after the
 * browser resumed would leave the file downloaded twice.
 */
export const NATIVE_TIMEOUT_MS = 20_000;

/** The flavor of this build: which host it talks to. */
export type Flavor = "release" | "development";

/** The host name a flavor talks to. */
export function hostName(flavor: Flavor): string {
  return flavor === "release" ? HOST_NAME : DEV_HOST_NAME;
}

/** `runtime.sendNativeMessage`, injected so the logic runs without a browser. */
export type SendNative = (host: string, message: Envelope) => Promise<unknown>;

/** Why no reply came back. */
export type TransportFailure =
  /** No host is registered under the name (Boltget is not installed). */
  | "not_installed"
  /** The host's manifest does not list this extension. */
  | "forbidden"
  /** The host started and exited without a reply. */
  | "host_exited"
  /** No reply within {@link NATIVE_TIMEOUT_MS}. */
  | "timeout"
  /** A reply that is not one of the protocol's. */
  | "invalid_reply"
  /** Anything else the browser reported. */
  | "other";

/** A reply, or why there is none. */
export type NativeResult =
  ReplyEnvelope | { type: "failed"; reason: TransportFailure; detail: string };

/** Sorts the browser's error text into a {@link TransportFailure}. */
export function classifyError(message: string): TransportFailure {
  const m = message.toLowerCase();
  // Chromium: "Specified native messaging host not found."
  // Firefox: "No such native application app.boltget.native_host".
  if (m.includes("not found") || m.includes("no such native application")) return "not_installed";
  // Chromium: "Access to the specified native messaging host is forbidden."
  if (m.includes("forbidden")) return "forbidden";
  // Chromium: "Native host has exited." / "Error when communicating with the
  // native messaging host."
  if (m.includes("exited") || m.includes("communicating")) return "host_exited";
  return "other";
}

const REPLY_TYPES = new Set(["hello", "accepted", "rejected", "status", "unavailable", "error"]);

/** Whether a value has the shape of a reply envelope. */
export function isReply(value: unknown): value is ReplyEnvelope {
  if (typeof value !== "object" || value === null) return false;
  const r = value as Record<string, unknown>;
  return typeof r.v === "number" && typeof r.type === "string" && REPLY_TYPES.has(r.type);
}

let nextId = Math.floor(Math.random() * 0x7fff_ffff);

/** A fresh message id (an integer below 2^32). */
export function messageId(): number {
  nextId = (nextId + 1) % 0xffff_ffff;
  return nextId;
}

/** Sends messages to one host. */
export class NativeClient {
  readonly #send: SendNative;
  readonly #host: string;
  readonly #timeout: number;

  constructor(send: SendNative, host: string, timeout = NATIVE_TIMEOUT_MS) {
    this.#send = send;
    this.#host = host;
    this.#timeout = timeout;
  }

  /** Sends one message and waits for its reply, or for why there is none. */
  async request(message: Message): Promise<NativeResult> {
    const id = messageId();
    const envelope = { v: VERSION, id, ...message } as Envelope;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<NativeResult>((resolve) => {
      timer = setTimeout(
        () => resolve({ type: "failed", reason: "timeout", detail: "no reply in time" }),
        this.#timeout,
      );
    });
    const reply = this.#send(this.#host, envelope).then(
      (value): NativeResult => {
        if (!isReply(value)) {
          return { type: "failed", reason: "invalid_reply", detail: "not a protocol reply" };
        }
        if (value.id !== undefined && value.id !== null && value.id !== id) {
          return { type: "failed", reason: "invalid_reply", detail: "reply for another message" };
        }
        return value;
      },
      (error: unknown): NativeResult => {
        const text = error instanceof Error ? error.message : String(error);
        return { type: "failed", reason: classifyError(text), detail: text };
      },
    );
    try {
      return await Promise.race([reply, timeout]);
    } finally {
      clearTimeout(timer);
    }
  }
}

/** What the popup and the options page show about the app. */
export type ConnectionStatus =
  | { state: "connected"; app: string; host: string; rules: CaptureRules | null }
  | { state: "not_running"; host: string }
  | { state: "not_installed" }
  | { state: "app_too_old" }
  | { state: "extension_too_old" }
  | { state: "mismatch" }
  | { state: "unreachable"; reason: string };

/** The connection status a `hello` result means (BROWSER-INTEGRATION.md, "Connection status"). */
export function connectionStatus(result: NativeResult): ConnectionStatus {
  switch (result.type) {
    case "hello":
      if (result.oldest > VERSION) return { state: "extension_too_old" };
      if (result.current < OLDEST_VERSION) return { state: "app_too_old" };
      return result.app === null
        ? { state: "not_running", host: result.host }
        : { state: "connected", app: result.app, host: result.host, rules: result.rules };
    case "error":
      if (result.reason === "app_too_old") return { state: "app_too_old" };
      if (result.reason === "extension_too_old") return { state: "extension_too_old" };
      return { state: "unreachable", reason: result.reason };
    case "unavailable":
      if (result.reason === "mismatch") return { state: "mismatch" };
      if (result.reason === "not_installed") return { state: "not_installed" };
      return { state: "unreachable", reason: result.reason };
    case "failed":
      if (result.reason === "not_installed") return { state: "not_installed" };
      return { state: "unreachable", reason: result.reason };
    default:
      return { state: "unreachable", reason: result.type };
  }
}
