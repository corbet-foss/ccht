/** Typed mirrors of the Rust auth surface (`ccht::auth`) for web consumers.
 *
 * The conversation model stays in Rust/Wasm; these types only describe *who*
 * may act. Transport, login ceremonies, credential storage, polling and
 * product copy belong to the embedding application. Nothing here touches the
 * DOM, performs network I/O, or spawns processes: effects run in the
 * application backend and reach the browser through app-supplied async
 * callbacks and explicit service keys.
 */

/** Login state of one provider, mirroring `ccht::auth::AuthState`.
 *
 * The shape round-trips the Rust `snake_case` JSON byte-for-byte, so a state
 * serialized by Rust deserializes here and vice versa:
 * - `Unknown` becomes the string `"unknown"`.
 * - `Unauthenticated` becomes the string `"unauthenticated"`.
 * - `Authenticated { account }` becomes `{"authenticated": {"account": ...}}`,
 *   where `account` is a display-only label (string), `null`, or absent.
 *   The label is provenance for the UI and never an authentication proof.
 */
export type AuthState = 'unknown' | 'unauthenticated' | { authenticated: { account?: string | null } };

/** Whether routine work may proceed without prompting for login first. Mirrors `AuthState::authenticated`. */
export function isAuthenticated(state: AuthState): boolean {
  return typeof state === 'object' && state !== null && 'authenticated' in state;
}

/** Display-only account label (username, email, or subscription name), or null when absent. Never a secret. */
export function authAccount(state: AuthState): string | null {
  if (typeof state === 'object' && state !== null && 'authenticated' in state) {
    return state.authenticated.account ?? null;
  }
  return null;
}

/** Whether a decoded JSON value has the exact Rust `AuthState` shape. Unknown fields inside the inner object are ignored, matching serde's default. */
export function isAuthState(value: unknown): value is AuthState {
  try {
    parseAuthState(value);
    return true;
  } catch {
    return false;
  }
}

/** Validate a decoded JSON value as the Rust `AuthState` shape. Throws a `TypeError` when the shape differs. */
export function parseAuthState(value: unknown): AuthState {
  if (value === 'unknown' || value === 'unauthenticated') {
    return value;
  }
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    const keys = Object.keys(value);
    if (keys.length === 1 && keys[0] === 'authenticated') {
      const inner = (value as { authenticated: unknown }).authenticated;
      if (typeof inner === 'object' && inner !== null && !Array.isArray(inner)) {
        const account = (inner as { account?: unknown }).account;
        if (account === undefined) {
          return { authenticated: {} };
        }
        if (account === null || typeof account === 'string') {
          return { authenticated: { account } };
        }
      }
    }
  }
  throw new TypeError('invalid AuthState');
}

/** Device-code challenge as exchanged during a provider login ceremony.
 *
 * Field names stay `snake_case` to match the wire format applications pass
 * between their backend and the browser. The backend remains authoritative:
 * it issues the challenge, pins any provider-specific endpoint, and polls
 * for completion. The browser only displays a validated challenge and never
 * invents one.
 */
export interface Challenge {
  verification_url: string;
  user_code: string;
}

const CHALLENGE_CODE_PATTERN = /^[A-Za-z0-9-]+$/;
const CHALLENGE_CODE_MAX_LENGTH = 64;

/** Whether a value is a renderable device-code challenge.
 *
 * Accepts any `https:` URL with a host (applications may narrow this
 * further, for example by pinning the provider's device endpoint) and a
 * non-empty code of up to 64 ASCII alphanumeric or `-` characters. URLs with
 * embedded credentials are rejected. Never throws; validate before rendering
 * a challenge as a link or code.
 */
export function validateChallenge(value: unknown): value is Challenge {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return false;
  }
  const { verification_url, user_code } = value as Record<string, unknown>;
  if (typeof verification_url !== 'string' || typeof user_code !== 'string') {
    return false;
  }
  if (user_code.length < 1 || user_code.length > CHALLENGE_CODE_MAX_LENGTH) {
    return false;
  }
  if (!CHALLENGE_CODE_PATTERN.test(user_code)) {
    return false;
  }
  let url: URL;
  try {
    url = new URL(verification_url);
  } catch {
    return false;
  }
  if (url.protocol !== 'https:' || url.hostname === '') {
    return false;
  }
  if (url.username !== '' || url.password !== '') {
    return false;
  }
  return true;
}

/** Opaque credential, mirroring `ccht::auth::Credential`.
 *
 * Stores treat both fields as opaque. `account` is display-only; `secret`
 * holds the raw secret bytes. Encode at the application boundary (for
 * example with `TextEncoder` or base64) when the backing store needs text,
 * and never write secrets to logs, errors, or fixtures.
 */
export interface Credential {
  account: string;
  secret: Uint8Array;
}

/** Application-supplied credential persistence, mirroring `ccht::auth::CredentialsProvider`.
 *
 * One method triple per service key. Service keys are application-defined
 * opaque strings (never ambient authority); native applications back this
 * with the OS keychain and web applications with origin-scoped browser
 * storage. Removing a missing entry succeeds. Implementations reject the
 * promise when the backing store fails.
 */
export interface CredentialsProvider {
  /** Read stored credentials, if any, for a service key. */
  readCredentials(service: string): Promise<Credential | null>;
  /** Persist credentials for a service key, replacing any previous entry. */
  writeCredentials(service: string, credential: Credential): Promise<void>;
  /** Remove stored credentials for a service key. Missing entries are not an error. */
  deleteCredentials(service: string): Promise<void>;
}
