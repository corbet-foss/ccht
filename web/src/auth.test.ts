/** Contract tests for the web auth mirrors (web/src/auth.ts).
 *
 * Covers the Rust `AuthState` shape validation, the authenticated/account
 * projections, and the render-gate for device-code challenges. Fixtures
 * only; no DOM, network, or storage.
 */
import { describe, expect, test } from 'bun:test';
import { authAccount, isAuthenticated, isAuthState, parseAuthState, validateChallenge } from './auth.ts';

describe('isAuthenticated', () => {
  test('only the authenticated object shape counts', () => {
    expect(isAuthenticated('unknown')).toBe(false);
    expect(isAuthenticated('unauthenticated')).toBe(false);
    expect(isAuthenticated({ authenticated: {} })).toBe(true);
    expect(isAuthenticated({ authenticated: { account: null } })).toBe(true);
    expect(isAuthenticated({ authenticated: { account: 'julian' } })).toBe(true);
  });
});

describe('authAccount', () => {
  test('returns the display label or null', () => {
    expect(authAccount('unknown')).toBeNull();
    expect(authAccount('unauthenticated')).toBeNull();
    expect(authAccount({ authenticated: {} })).toBeNull();
    expect(authAccount({ authenticated: { account: null } })).toBeNull();
    expect(authAccount({ authenticated: { account: 'julian' } })).toBe('julian');
  });
});

describe('parseAuthState / isAuthState', () => {
  test('accepts the exact Rust JSON shapes', () => {
    expect(parseAuthState('unknown')).toBe('unknown');
    expect(parseAuthState('unauthenticated')).toBe('unauthenticated');
    expect(parseAuthState({ authenticated: {} })).toEqual({ authenticated: {} });
    expect(parseAuthState({ authenticated: { account: null } })).toEqual({
      authenticated: { account: null },
    });
    expect(parseAuthState({ authenticated: { account: 'a' } })).toEqual({
      authenticated: { account: 'a' },
    });
    expect(parseAuthState(JSON.parse(JSON.stringify({ authenticated: { account: 'a' } })))).toEqual({
      authenticated: { account: 'a' },
    });
  });

  test('ignores unknown fields inside the inner object like serde', () => {
    expect(parseAuthState({ authenticated: { account: 'a', extra: 1 } })).toEqual({
      authenticated: { account: 'a' },
    });
  });

  test('rejects everything else with a TypeError', () => {
    for (const value of [
      null,
      undefined,
      0,
      true,
      [],
      {},
      { authenticated: null },
      { authenticated: 42 },
      { authenticated: { account: 42 } },
      { authenticated: { account: {} } },
      { unknown: {} },
      { authenticated: {}, extra: 1 },
      'UNKNOWN',
      'authenticated',
    ]) {
      expect(() => parseAuthState(value)).toThrow(TypeError);
      expect(isAuthState(value)).toBe(false);
    }
    expect(isAuthState('unknown')).toBe(true);
    expect(isAuthState({ authenticated: { account: 'a' } })).toBe(true);
  });

  test('never throws on hostile input', () => {
    expect(() => isAuthState(Object.create(null))).not.toThrow();
  });
});

describe('validateChallenge', () => {
  const good = { verification_url: 'https://auth.openai.com/codex/device', user_code: 'TEST-4826' };

  test('accepts a well-formed challenge', () => {
    expect(validateChallenge(good)).toBe(true);
    expect(validateChallenge({ ...good, user_code: 'a'.repeat(64) })).toBe(true);
    expect(validateChallenge({ ...good, verification_url: 'https://example.com/x?y=1' })).toBe(true);
    expect(validateChallenge({ ...good, extra: 'ignored' })).toBe(true);
  });

  test('rejects malformed challenges without throwing', () => {
    for (const value of [
      null,
      undefined,
      'code',
      [],
      {},
      { verification_url: good.verification_url },
      { user_code: good.user_code },
      { verification_url: 42, user_code: good.user_code },
      { ...good, user_code: '' },
      { ...good, user_code: 'a'.repeat(65) },
      { ...good, user_code: 'has space' },
      { ...good, user_code: 'semi;colon' },
      { ...good, user_code: 'uniçode' },
      { ...good, verification_url: 'http://auth.openai.com/codex/device' },
      { ...good, verification_url: 'not a url' },
      { ...good, verification_url: 'https://user:pass@auth.openai.com/' },
      { ...good, verification_url: 'https://' },
      { ...good, verification_url: 'javascript:alert(1)' },
    ]) {
      expect(validateChallenge(value)).toBe(false);
    }
  });
});
