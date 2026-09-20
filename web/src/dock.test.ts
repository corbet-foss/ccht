/** Conformance tests for the framework-free dock state (web/src/dock.ts).
 *
 * These mirror the Rust `src/dock.rs` unit tests case-for-case: both
 * implementations share the id rules, error names, open/placement
 * semantics, focus-token lifecycle, and serialization contract. Fixtures
 * only; no DOM, network, or storage.
 */
import { describe, expect, test } from 'bun:test';
import { createDockManager, validateDockId } from './dock.ts';

describe('validateDockId', () => {
  test('accepts lowercase, digits, dash, underscore', () => {
    for (const id of ['a', 'scope', 'models-panel', 'chat_2', 'a-b_c9']) {
      expect(validateDockId(id)).toBeNull();
    }
  });

  test('accepts exactly 64 characters', () => {
    expect(validateDockId('a'.repeat(64))).toBeNull();
  });

  test('rejects empty, non-string, too long, and bad characters', () => {
    expect(validateDockId('')).not.toBeNull();
    expect(validateDockId('a'.repeat(65))).not.toBeNull();
    for (const id of ['Scope', 'has space', 'dot.name', 'slash/x', 'uniçode', 'CAPS'] as unknown[]) {
      expect(validateDockId(id)).not.toBeNull();
    }
    for (const id of [null, undefined, 42, true, {}, []] as unknown[]) {
      expect(validateDockId(id)).not.toBeNull();
    }
  });

  test('never throws', () => {
    expect(() => validateDockId(Symbol('x') as unknown as string)).not.toThrow();
  });
});

describe('register', () => {
  test('registers closed docks of every kind and placement', () => {
    const docks = createDockManager();
    docks.register('chat', 'chat', 'left');
    docks.register('cfg', 'config', 'right');
    docks.register('misc', 'custom', 'bottom');
    docks.register('inline', 'custom', 'inline');
    expect(docks.isOpen('chat')).toBe(false);
    expect(docks.placement('cfg')).toBe('right');
  });

  test('rejects duplicates, malformed ids, kinds, and placements', () => {
    const docks = createDockManager();
    docks.register('scope', 'config', 'left');
    expect(() => docks.register('scope', 'config', 'left')).toThrow('DuplicateDock: scope');
    expect(() => docks.register('', 'config', 'left')).toThrow();
    expect(() => docks.register('other', 'bogus' as never, 'left')).toThrow('invalid dock kind');
    expect(() => docks.register('other', 'config', 'top' as never)).toThrow('invalid dock placement');
  });
});

describe('open/close/toggle', () => {
  test('open and close are idempotent; toggle flips and returns state', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    docks.open('a');
    docks.open('a');
    expect(docks.isOpen('a')).toBe(true);
    expect(docks.toggle('a')).toBe(false);
    expect(docks.toggle('a')).toBe(true);
    docks.close('a');
    docks.close('a');
    expect(docks.isOpen('a')).toBe(false);
  });

  test('unknown ids throw UnknownDock on every accessor', () => {
    const docks = createDockManager();
    for (const fn of [
      () => docks.open('nope'),
      () => docks.openWithFocus('nope', 't'),
      () => docks.close('nope'),
      () => docks.toggle('nope'),
      () => docks.isOpen('nope'),
      () => docks.placement('nope'),
      () => docks.setPlacement('nope', 'left'),
    ]) {
      expect(fn).toThrow('UnknownDock: nope');
    }
  });

  test('docks are independent and openDocks preserves registration order', () => {
    const docks = createDockManager();
    docks.register('one', 'chat', 'left');
    docks.register('two', 'config', 'right');
    docks.register('three', 'custom', 'inline');
    docks.open('three');
    docks.open('one');
    expect(docks.openDocks()).toEqual(['one', 'three']);
    docks.close('one');
    expect(docks.openDocks()).toEqual(['three']);
  });

  test('setPlacement moves only the target dock', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    docks.register('b', 'chat', 'left');
    docks.setPlacement('a', 'bottom');
    expect(docks.placement('a')).toBe('bottom');
    expect(docks.placement('b')).toBe('left');
    expect(() => docks.setPlacement('a', 'nowhere' as never)).toThrow('invalid dock placement');
  });

  test('closeAll closes everything but keeps the staged token', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    docks.register('b', 'config', 'right');
    docks.openWithFocus('a', 'tok');
    docks.open('b');
    docks.closeAll();
    expect(docks.openDocks()).toEqual([]);
    expect(docks.takeFocusToken()).toBe('tok');
  });
});

describe('focus token', () => {
  test('openWithFocus stages, plain open preserves, take consumes once', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    expect(docks.takeFocusToken()).toBeNull();
    docks.openWithFocus('a', 'first');
    docks.open('a');
    expect(docks.takeFocusToken()).toBe('first');
    expect(docks.takeFocusToken()).toBeNull();
    docks.openWithFocus('a', 'second');
    docks.close('a');
    expect(docks.takeFocusToken()).toBe('second');
  });

  test('empty tokens are rejected before lookup', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    expect(() => docks.openWithFocus('a', '')).toThrow('invalid focus token');
    expect(() => docks.openWithFocus('ghost', '')).toThrow('invalid focus token');
    expect(docks.takeFocusToken()).toBeNull();
  });

  test('managers are independent', () => {
    const first = createDockManager();
    const second = createDockManager();
    first.register('a', 'chat', 'left');
    second.register('a', 'chat', 'left');
    first.openWithFocus('a', 'tok');
    expect(second.isOpen('a')).toBe(false);
    expect(second.takeFocusToken()).toBeNull();
  });
});

describe('serialize/restore', () => {
  test('round-trips open state and placement without the focus token', () => {
    const docks = createDockManager();
    docks.register('scope', 'config', 'left');
    docks.register('models', 'config', 'right');
    docks.openWithFocus('scope', 'tok');
    docks.setPlacement('models', 'bottom');
    const snapshot = docks.serialize();
    expect(snapshot).not.toContain('tok');
    const revived = createDockManager();
    revived.restore(snapshot);
    expect(revived.isOpen('scope')).toBe(true);
    expect(revived.isOpen('models')).toBe(false);
    expect(revived.placement('models')).toBe('bottom');
    expect(revived.takeFocusToken()).toBeNull();
  });

  test('empty managers round-trip', () => {
    const revived = createDockManager();
    revived.restore(createDockManager().serialize());
    expect(revived.openDocks()).toEqual([]);
  });

  test('malformed snapshots throw and leave state untouched', () => {
    const malformed = [
      'not json',
      '{}',
      '[null]',
      '["scope"]',
      '[{}]',
      '[{"id":"","kind":"config","placement":"left","open":false}]',
      '[{"id":"UPPER","kind":"config","placement":"left","open":false}]',
      '[{"id":"a","kind":"bogus","placement":"left","open":false}]',
      '[{"id":"a","kind":"config","placement":"top","open":false}]',
      '[{"id":"a","kind":"config","placement":"left"}]',
      '[{"id":"a","kind":"config","placement":"left","open":"yes"}]',
      '[{"id":"a","kind":"config","placement":"left","open":false},{"id":"a","kind":"chat","placement":"right","open":true}]',
    ];
    for (const snapshot of malformed) {
      const docks = createDockManager();
      docks.register('keep', 'chat', 'left');
      docks.open('keep');
      expect(() => docks.restore(snapshot)).toThrow(/^invalid dock snapshot/);
      expect(docks.isOpen('keep')).toBe(true);
      expect(docks.openDocks()).toEqual(['keep']);
    }
  });

  test('restore replaces everything and clears the staged token', () => {
    const docks = createDockManager();
    docks.register('stale', 'chat', 'left');
    docks.openWithFocus('stale', 'tok');
    docks.restore('[{"id":"fresh","kind":"custom","placement":"inline","open":true}]');
    expect(() => docks.isOpen('stale')).toThrow('UnknownDock: stale');
    expect(docks.isOpen('fresh')).toBe(true);
    expect(docks.takeFocusToken()).toBeNull();
  });

  test('error messages never carry focus tokens', () => {
    const docks = createDockManager();
    docks.register('a', 'chat', 'left');
    docks.openWithFocus('a', 'secret-token');
    for (const fn of [() => docks.open('ghost'), () => docks.toggle('ghost'), () => docks.restore('[1]')]) {
      try {
        fn();
        expect.unreachable();
      } catch (error) {
        expect(String(error)).not.toContain('secret-token');
      }
    }
  });
});
