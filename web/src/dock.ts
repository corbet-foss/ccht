/** Framework-free dock state for web consumers, mirroring the dock vocabulary.
 *
 * Docks are named regions of application chrome (for example a chat rail or
 * a configuration panel). `DockKind` classifies a dock, `Placement` places
 * it, and the manager tracks which docks exist, which are open, and where
 * each one sits. The conversation model stays in Rust/Wasm; these types only
 * describe *where* a surface lives and whether it is visible.
 *
 * This module is plain TypeScript: no DOM access, no network I/O, no spawned
 * processes, and no storage. Persistence travels through `serialize` /
 * `restore` as explicit JSON handled by the application; focus is handed off
 * through an opaque token the application moves to its own focus target.
 * Product copy belongs to the embedding application.
 */

/** What a dock is for. `chat` hosts a conversation surface, `config` hosts
 * settings, and `custom` covers every application-defined use. */
export type DockKind = 'chat' | 'config' | 'custom';

/** Where a dock sits. Edge values pin the dock to a window edge; `inline`
 * renders it in the normal application flow instead. */
export type Placement = 'left' | 'right' | 'bottom' | 'inline';

const DOCK_ID_PATTERN = /^[a-z0-9-_]+$/;
const DOCK_ID_MAX_LENGTH = 64;

/** Validate a dock id. Returns an error message, or null when the id is valid.
 *
 * Rules: a non-empty string of at most 64 ASCII `[a-z0-9-_]` characters.
 * Never throws; validate before registering or restoring a dock.
 */
export function validateDockId(id: unknown): string | null {
  if (typeof id !== 'string' || id.length === 0) {
    return 'invalid dock id: must be a non-empty string';
  }
  if (id.length > DOCK_ID_MAX_LENGTH) {
    return 'invalid dock id: must be at most 64 characters';
  }
  if (!DOCK_ID_PATTERN.test(id)) {
    return 'invalid dock id: must match [a-z0-9-_]';
  }
  return null;
}

/** One persisted dock entry. `open` is the only mutable runtime state; the
 * focus token is deliberately absent and never survives serialization. */
export interface DockEntry {
  id: string;
  kind: DockKind;
  placement: Placement;
  open: boolean;
}

/** Named dock registry with open state, placement, and focus handoff.
 *
 * Methods that name a dock throw `Error('UnknownDock: <id>')` for ids that
 * were never registered. `register` throws `Error('DuplicateDock: <id>')`
 * for an id that already exists, and rethrows the `validateDockId` message
 * for a malformed id. Invalid kinds, placements, focus tokens, and snapshots
 * throw `Error` with a fixed `invalid ...` message.
 */
export interface DockManager {
  /** Register a closed dock. Throws on a malformed id, an invalid kind or
   * placement, or a duplicate id. */
  register(id: string, kind: DockKind, placement: Placement): void;
  /** Mark a dock open. Throws `UnknownDock` for an unregistered id. */
  open(id: string): void;
  /** Mark a dock open and stage a focus token for the application to consume
   * with `takeFocusToken`. The token must be a non-empty string. */
  openWithFocus(id: string, token: string): void;
  /** Mark a dock closed. Throws `UnknownDock` for an unregistered id. */
  close(id: string): void;
  /** Flip a dock's open state and return the new state. */
  toggle(id: string): boolean;
  /** Whether a dock is currently open. */
  isOpen(id: string): boolean;
  /** A dock's current placement. */
  placement(id: string): Placement;
  /** Move a dock to another placement. */
  setPlacement(id: string, placement: Placement): void;
  /** Ids of open docks, in registration order. */
  openDocks(): string[];
  /** Take the staged focus token once, clearing it; null when none is staged. */
  takeFocusToken(): string | null;
  /** Mark every dock closed. Staged focus tokens are left untouched. */
  closeAll(): void;
  /** Persist `[{id, kind, placement, open}]` as JSON, in registration order.
   * The focus token is never persisted. */
  serialize(): string;
  /** Replace all state from `serialize` output. Validates every entry first,
   * so a malformed snapshot leaves the current state untouched, and clears
   * any staged focus token. */
  restore(json: string): void;
}

/** Create an empty dock manager. Managers are independent; applications that
 * need shared state pass one manager around instead of creating several. */
export function createDockManager(): DockManager {
  const docks = new Map<string, { kind: DockKind; placement: Placement; open: boolean }>();
  let focusToken: string | null = null;

  function entry(id: string): { kind: DockKind; placement: Placement; open: boolean } {
    const found = docks.get(id);
    if (found === undefined) {
      throw new Error(`UnknownDock: ${id}`);
    }
    return found;
  }

  function checkKind(kind: unknown): asserts kind is DockKind {
    if (kind !== 'chat' && kind !== 'config' && kind !== 'custom') {
      throw new Error('invalid dock kind');
    }
  }

  function checkPlacement(value: unknown): asserts value is Placement {
    if (value !== 'left' && value !== 'right' && value !== 'bottom' && value !== 'inline') {
      throw new Error('invalid dock placement');
    }
  }

  function checkToken(token: unknown): asserts token is string {
    if (typeof token !== 'string' || token.length === 0) {
      throw new Error('invalid focus token');
    }
  }

  return {
    register(id, kind, placement) {
      const invalid = validateDockId(id);
      if (invalid !== null) {
        throw new Error(invalid);
      }
      checkKind(kind);
      checkPlacement(placement);
      if (docks.has(id)) {
        throw new Error(`DuplicateDock: ${id}`);
      }
      docks.set(id, { kind, placement, open: false });
    },
    open(id) {
      entry(id).open = true;
    },
    openWithFocus(id, token) {
      checkToken(token);
      entry(id).open = true;
      focusToken = token;
    },
    close(id) {
      entry(id).open = false;
    },
    toggle(id) {
      const target = entry(id);
      target.open = !target.open;
      return target.open;
    },
    isOpen(id) {
      return entry(id).open;
    },
    placement(id) {
      return entry(id).placement;
    },
    setPlacement(id, placement) {
      checkPlacement(placement);
      entry(id).placement = placement;
    },
    openDocks() {
      const ids: string[] = [];
      for (const [id, state] of docks) {
        if (state.open) {
          ids.push(id);
        }
      }
      return ids;
    },
    takeFocusToken() {
      const token = focusToken;
      focusToken = null;
      return token;
    },
    closeAll() {
      for (const state of docks.values()) {
        state.open = false;
      }
    },
    serialize() {
      const snapshot: DockEntry[] = [];
      for (const [id, state] of docks) {
        snapshot.push({ id, kind: state.kind, placement: state.placement, open: state.open });
      }
      return JSON.stringify(snapshot);
    },
    restore(json) {
      let decoded: unknown;
      try {
        decoded = JSON.parse(json);
      } catch {
        throw new Error('invalid dock snapshot: not JSON');
      }
      if (!Array.isArray(decoded)) {
        throw new Error('invalid dock snapshot: must be an array');
      }
      const next = new Map<string, { kind: DockKind; placement: Placement; open: boolean }>();
      for (const value of decoded) {
        if (typeof value !== 'object' || value === null || Array.isArray(value)) {
          throw new Error('invalid dock snapshot: entry must be an object');
        }
        const record = value as Record<string, unknown>;
        const invalid = validateDockId(record.id);
        if (invalid !== null) {
          throw new Error(`invalid dock snapshot: ${invalid}`);
        }
        const id = record.id as string;
        if (record.kind !== 'chat' && record.kind !== 'config' && record.kind !== 'custom') {
          throw new Error('invalid dock snapshot: invalid dock kind');
        }
        if (
          record.placement !== 'left' &&
          record.placement !== 'right' &&
          record.placement !== 'bottom' &&
          record.placement !== 'inline'
        ) {
          throw new Error('invalid dock snapshot: invalid dock placement');
        }
        if (typeof record.open !== 'boolean') {
          throw new Error('invalid dock snapshot: open must be a boolean');
        }
        if (next.has(id)) {
          throw new Error('invalid dock snapshot: duplicate dock id');
        }
        next.set(id, {
          kind: record.kind,
          placement: record.placement,
          open: record.open,
        });
      }
      docks.clear();
      for (const [id, state] of next) {
        docks.set(id, state);
      }
      focusToken = null;
    },
  };
}
