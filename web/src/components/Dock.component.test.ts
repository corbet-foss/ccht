/** Browser behavior of the shared edge rail (Dock.svelte).
 *
 * jsdom exercises tab/panel switching, Esc handling, focus move and
 * restore, and the inert contract. Fixtures only; no network or storage.
 */
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import { afterEach, describe, expect, test, vi } from 'vitest';
import Dock from './Dock.svelte';

afterEach(() => cleanup());

function openDock(props: Record<string, unknown> = {}) {
  return render(Dock, {
    side: 'left',
    title: 'Scope',
    open: false,
    onClose: () => {},
    onOpen: () => {},
    tabLabel: 'Scope',
    tabSummary: '3 checks',
    panelId: 'scope-panel',
    ...props,
  });
}

describe('closed rail', () => {
  test('shows the edge tab with summary and hides the panel', () => {
    const { getByRole, container } = openDock();
    const tab = getByRole('button', { name: 'Open Scope: 3 checks' });
    expect(tab.getAttribute('aria-expanded')).toBe('false');
    expect(tab.getAttribute('aria-controls')).toBe('scope-panel');
    const panel = container.querySelector('#scope-panel');
    expect(panel?.getAttribute('aria-hidden')).toBe('true');
    expect(panel?.getAttribute('data-open')).toBe('false');
  });

  test('tab click calls onOpen', async () => {
    const onOpen = vi.fn();
    const { getByRole } = openDock({ onOpen });
    await fireEvent.click(getByRole('button', { name: 'Open Scope: 3 checks' }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });
});

describe('open rail', () => {
  test('renders dialog content and Done/Close affordances', () => {
    const { getByRole } = openDock({ open: true });
    const dialog = getByRole('dialog', { name: 'Scope' });
    expect(dialog.getAttribute('data-open')).toBe('true');
    expect(getByRole('button', { name: 'Close Scope' })).toBeTruthy();
  });

  test('Escape calls onClose only when open', async () => {
    const onClose = vi.fn();
    const closed = openDock({ onClose });
    await fireEvent.keyDown(closed.container, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
    const opened = openDock({ open: true, onClose });
    await fireEvent.keyDown(opened.container.ownerDocument, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test('opening moves focus into the panel, closing returns it', async () => {
    // jsdom has no layout: every getClientRects() is empty, so the filter
    // correctly falls back to the panel itself. Stub rects to exercise the
    // real-browser path where the first control wins.
    const rects = vi.spyOn(window.Element.prototype, 'getClientRects').mockReturnValue([{} as DOMRect]);
    try {
      const { getByRole, rerender } = openDock();
      getByRole('button', { name: 'Open Scope: 3 checks' }).focus();
      await rerender({ open: true });
      await new Promise((resolve) => requestAnimationFrame(() => resolve(undefined)));
      expect(document.activeElement?.getAttribute('aria-label')).toBe('Close Scope');
      await rerender({ open: false });
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined))));
      // The tab remounts on close; focus must land on the live node.
      const liveTab = getByRole('button', { name: 'Open Scope: 3 checks' });
      expect(liveTab.isConnected).toBe(true);
      expect(document.activeElement).toBe(liveTab);
    } finally {
      rects.mockRestore();
    }
  });

  test('opening falls back to the panel when no control is visible', async () => {
    const { getByRole, rerender, container } = openDock();
    await rerender({ open: true });
    await new Promise((resolve) => requestAnimationFrame(() => resolve(undefined)));
    expect(document.activeElement).toBe(container.querySelector('#scope-panel'));
    expect(getByRole('dialog', { name: 'Scope' })).toBeTruthy();
  });
});
