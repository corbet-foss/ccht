<!-- Product-neutral edge rail. The application owns all effects (dock state,
  persistence, focus targets beyond this panel); this component only renders
  the tab and panel chrome and forwards user intent through app-supplied
  open/close callbacks. It never fetches, spawns, or stores anything. -->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    side = 'left',
    title,
    open,
    onClose,
    onOpen,
    tabLabel,
    tabSummary = '',
    closeLabel,
    panelId,
    children
  }: {
    side: 'left' | 'right';
    title: string;
    open: boolean;
    onClose: () => void;
    onOpen: () => void;
    tabLabel: string;
    tabSummary?: string;
    closeLabel?: string;
    panelId: string;
    children: Snippet;
  } = $props();

  let panelEl: HTMLElement | undefined = $state(undefined);
  let tabEl: HTMLButtonElement | undefined = $state(undefined);
  let wasOpen = $state(false);
  let prevFocus: HTMLElement | null = $state(null);
  const dismissLabel = $derived(closeLabel ?? `Close ${title}`);

  function handleTabClick(event: MouseEvent) {
    prevFocus = event.currentTarget as HTMLElement;
    onOpen();
  }

  $effect(() => {
    if (open && !wasOpen) {
      wasOpen = true;
      prevFocus ??= document.activeElement as HTMLElement | null;
      requestAnimationFrame(() => {
        // First visible, enabled control wins; hidden or disabled nodes
        // never take focus. Fixed panels have no offsetParent, so filter
        // on client rects instead.
        const target = [...(panelEl?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])'
        ) ?? [])].find((node) => node.getClientRects().length > 0) ?? panelEl;
        target?.focus();
      });
    } else if (!open && wasOpen) {
      wasOpen = false;
      // Prefer the live remounted tab over the recorded invoker: the tab
      // unmounts while the panel is open, so a stored node may be stale.
      const invoker = prevFocus?.isConnected ? prevFocus : null;
      prevFocus = null;
      requestAnimationFrame(() => {
        const liveTab = tabEl?.isConnected ? tabEl : null;
        (liveTab ?? invoker)?.focus?.();
      });
    }
  });

  $effect(() => {
    if (panelEl) (panelEl as HTMLElement & { inert: boolean }).inert = !open;
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && open) onClose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if !open}
  <button
    bind:this={tabEl}
    type="button"
    class="ccht-dock-tab"
    data-side={side}
    aria-expanded="false"
    aria-controls={panelId}
    aria-label={tabSummary ? `Open ${title}: ${tabSummary}` : `Open ${title}`}
    onclick={handleTabClick}
  ><span class="ccht-dock-tab-text">{tabLabel} · {tabSummary || title}</span></button>
{/if}

<div
  bind:this={panelEl}
  id={panelId}
  class="ccht-dock-panel"
  data-side={side}
  data-open={open ? 'true' : 'false'}
  role="dialog"
  aria-label={title}
  aria-hidden={!open}
  tabindex="-1"
>
  <div class="ccht-dock-head">
    <div class="ccht-dock-title"><h2>{title}</h2>{#if tabSummary}<p>{tabSummary}</p>{/if}</div>
    <button type="button" class="ccht-dock-close" aria-label={dismissLabel} onclick={onClose}>Close</button>
  </div>
  <div class="ccht-dock-body">{#if children}{@render children()}{/if}</div>
</div>

<style>
  .ccht-dock-tab {
    position: fixed; top: 50%; z-index: 80; transform: translateY(-50%);
    max-height: min(60vh, 30rem); max-width: 2.4rem; padding: 0.7rem 0.4rem;
    display: flex; align-items: center; justify-content: center;
    border: 1px solid var(--ccht-border, #30405c);
    color: var(--ccht-fg, #c2cede);
    background: var(--ccht-tab-bg, #111c2df2);
    font: 800 0.68rem ui-monospace, monospace; cursor: pointer;
    writing-mode: vertical-rl; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .ccht-dock-tab[data-side='left'] { left: 0; border-left: 0; border-radius: 0 0.5rem 0.5rem 0; }
  .ccht-dock-tab[data-side='right'] { right: 0; border-right: 0; border-radius: 0.5rem 0 0 0.5rem; }
  .ccht-dock-tab:hover {
    border-color: var(--ccht-accent, #53708f);
    color: var(--ccht-fg-bright, #e2eaf5);
    background: var(--ccht-tab-bg-hover, #17243af2);
  }
  .ccht-dock-tab-text { overflow: hidden; text-overflow: ellipsis; }
  .ccht-dock-panel {
    position: fixed; top: 0; bottom: 0; z-index: 70;
    width: min(30rem, 100vw); display: flex; flex-direction: column;
    background: var(--ccht-panel-bg, #0d1625);
    color: var(--ccht-fg, #d8e2f0); visibility: hidden;
    transition: transform 0.2s ease, visibility 0s linear 0.2s;
  }
  .ccht-dock-panel[data-side='left'] {
    left: 0; border-right: 1px solid var(--ccht-border, #223049); transform: translateX(-102%);
  }
  .ccht-dock-panel[data-side='right'] {
    right: 0; border-left: 1px solid var(--ccht-border, #223049); transform: translateX(102%);
  }
  .ccht-dock-panel[data-open='true'] { transform: none; visibility: visible; transition: transform 0.2s ease; }
  .ccht-dock-panel[data-open='false'] { visibility: hidden; }
  .ccht-dock-head {
    display: flex; align-items: center; justify-content: space-between; gap: 0.8rem;
    padding: 1rem 1.1rem; border-bottom: 1px solid var(--ccht-border, #223049);
  }
  .ccht-dock-title { min-width: 0; }
  .ccht-dock-title h2 { margin: 0; font-size: 1.02rem; overflow-wrap: anywhere; }
  .ccht-dock-title p {
    margin: 0.3rem 0 0; overflow-wrap: anywhere;
    color: var(--ccht-muted, #8fa0b7); font: 750 0.68rem ui-monospace, monospace;
  }
  .ccht-dock-close {
    flex: 0 0 auto; min-height: 2.5rem; padding: 0.55rem 0.85rem;
    border: 1px solid var(--ccht-border, #30405c); border-radius: 0.5rem;
    color: var(--ccht-fg, #c2cede);
    background: var(--ccht-tab-bg, #111c2d);
    font-size: 0.72rem; font-weight: 850; cursor: pointer;
  }
  .ccht-dock-close:hover {
    border-color: var(--ccht-accent, #53708f);
    background: var(--ccht-tab-bg-hover, #17243a);
  }
  .ccht-dock-body {
    flex: 1; min-height: 0; overflow: auto; padding: 1rem 1.1rem;
    display: grid; gap: 0.85rem; align-content: start;
  }
  @media (max-width: 560px) {
    .ccht-dock-panel { width: 100vw; }
    .ccht-dock-head, .ccht-dock-body { padding-left: 0.85rem; padding-right: 0.85rem; }
  }
  @media (prefers-reduced-motion: reduce) { .ccht-dock-panel { transition: none; } }
</style>
