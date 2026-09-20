<!-- Generic per-step backend/model/options config form (content only).
  The application wraps this in a Dock and owns all effects (loading model
  lists, refreshing controls, connecting accounts); this component only
  renders one fieldset per step and forwards user intent through
  app-supplied callbacks. It never fetches, spawns, or stores anything. -->
<script module lang="ts">
  import type { SessionConfigOption } from '../../index.js';

  export interface StepBackendOption {
    value: string;
    name: string;
  }

  export interface StepModelOption {
    value: string;
    name: string;
  }

  export interface StepData {
    id: string;
    label: string;
    backendValue: string;
    backendOptions: StepBackendOption[];
    backendPlaceholder: string;
    accountConnected: boolean;
    accountBusy: boolean;
    configurationError?: string | null;
    modelValue: string;
    modelOptions: StepModelOption[];
    refreshable: boolean;
    extraOptions: SessionConfigOption[];
    extraValues: Record<string, string | boolean>;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import type {
    ConfigChoice,
    ConfigGroup,
    SessionConfigOption
  } from '../../index.js';

  let {
    steps,
    refreshing = false,
    onSelectBackend,
    onSelectModel,
    onSelectOption,
    onRefresh,
    accountSlot,
    headerCopy,
    footerSlot
  }: {
    steps: StepData[];
    refreshing?: boolean;
    onSelectBackend: (id: string, value: string) => void;
    onSelectModel: (id: string, value: string) => void;
    onSelectOption: (
      id: string,
      option: SessionConfigOption,
      value: string | boolean
    ) => void;
    onRefresh: (id: string) => void;
    accountSlot?: Snippet<[{ step: string }]>;
    headerCopy?: Snippet;
    footerSlot?: Snippet;
  } = $props();

  // Flatten grouped select choices while preserving order.
  function choices(option: SessionConfigOption | undefined): ConfigChoice[] {
    if (option?.type !== 'select') return [];
    return option.options.flatMap((entry) =>
      'options' in entry
        ? (entry as ConfigGroup).options
        : [entry as ConfigChoice]
    );
  }

  // Drop the model control defensively; it has its own dedicated row.
  function extraOptions(step: StepData): SessionConfigOption[] {
    return step.extraOptions.filter(
      (item) => item.category !== 'model' && item.id !== 'model'
    );
  }

  function selectedValue(
    step: StepData,
    option: SessionConfigOption
  ): string | boolean {
    return step.extraValues[option.id] ?? option.currentValue;
  }
</script>

{#if headerCopy}
  <div class="ccht-steps-copy">{@render headerCopy()}</div>
{/if}

{#each steps as step (step.id)}
  <fieldset class="ccht-step">
    <legend>{step.label}</legend>
    <label class="ccht-step-label" for={`ccht-step-${step.id}-assistant`}>
      {step.label} assistant
    </label>
    <select
      id={`ccht-step-${step.id}-assistant`}
      class="ccht-step-select"
      value={step.backendValue}
      disabled={step.accountBusy}
      onchange={(event) => onSelectBackend(step.id, event.currentTarget.value)}
    >
      <option value="">{step.backendPlaceholder}</option>
      {#each step.backendOptions as backend (backend.value)}
        <option value={backend.value}>{backend.name}</option>
      {/each}
    </select>
    {#if accountSlot && !step.accountConnected}
      {@render accountSlot({ step: step.id })}
    {:else}
      <label class="ccht-step-label" for={`ccht-step-${step.id}-model`}>
        {step.label} model
      </label>
      <select
        id={`ccht-step-${step.id}-model`}
        class="ccht-step-select"
        value={step.modelValue}
        onchange={(event) => onSelectModel(step.id, event.currentTarget.value)}
      >
        {#if step.modelOptions.length}
          {#each step.modelOptions as model (model.value)}
            <option value={model.value}>{model.name}</option>
          {/each}
        {:else if step.modelValue}
          <option value={step.modelValue}>{step.modelValue}</option>
        {:else}
          <option value="" disabled>No model</option>
        {/if}
      </select>
      {#each extraOptions(step) as item (item.id)}
        <label
          class="ccht-step-label"
          for={`ccht-step-${step.id}-option-${item.id}`}
          title={item.description}
        >
          {item.name}
        </label>
        {#if item.type === 'select'}
          <select
            id={`ccht-step-${step.id}-option-${item.id}`}
            class="ccht-step-select"
            value={String(selectedValue(step, item))}
            disabled={refreshing}
            onchange={(event) =>
              onSelectOption(step.id, item, event.currentTarget.value)}
          >
            {#each choices(item) as choice (choice.value)}
              <option value={choice.value}>{choice.name}</option>
            {/each}
          </select>
        {:else if item.type === 'boolean'}
          <input
            id={`ccht-step-${step.id}-option-${item.id}`}
            class="ccht-step-check"
            type="checkbox"
            checked={Boolean(selectedValue(step, item))}
            disabled={refreshing}
            onchange={(event) =>
              onSelectOption(step.id, item, event.currentTarget.checked)}
          />
        {/if}
      {/each}
      {#if step.accountConnected && step.refreshable}
        <button
          type="button"
          class="ccht-step-refresh"
          disabled={refreshing}
          onclick={() => onRefresh(step.id)}
        >
          {refreshing ? 'Loading controls…' : 'Refresh models'}
        </button>
        {#if step.configurationError}
          <span class="ccht-step-error" role="alert">
            {step.configurationError}
          </span>
        {/if}
      {/if}
    {/if}
  </fieldset>
{/each}

{#if footerSlot}
  {@render footerSlot()}
{/if}

<style>
  .ccht-steps-copy {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    color: var(--ccht-muted, #8fa0b7);
  }
  .ccht-step {
    display: grid;
    gap: 0.45rem;
    margin: 0;
    border: 1px solid var(--ccht-border, #30405c);
    border-radius: 0.5rem;
    padding: 0.75rem;
  }
  .ccht-step legend {
    font-weight: 700;
    padding: 0 0.4rem;
  }
  .ccht-step-label {
    font-size: 0.75rem;
    color: var(--ccht-muted, #8fa0b7);
  }
  .ccht-step-select {
    min-width: 0;
    width: 100%;
    padding: 0.5rem;
    border: 1px solid var(--ccht-select-border, #405779);
    border-radius: 0.4rem;
    background: var(--ccht-select-bg, #070f1b);
    color: var(--ccht-fg-bright, #e2eaf5);
  }
  .ccht-step-check {
    width: 1rem;
    min-height: 1rem;
    margin: 0;
    accent-color: var(--ccht-accent-check, #34d399);
  }
  .ccht-step-refresh {
    min-height: 2.2rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--ccht-border, #30405c);
    border-radius: 0.4rem;
    background: var(--ccht-tab-bg, #111c2d);
    color: var(--ccht-fg, #bdc8d7);
    font-size: 0.7rem;
    font-weight: 750;
    cursor: pointer;
  }
  .ccht-step-error {
    color: var(--ccht-error, #fda4af);
    font-size: 0.7rem;
  }
</style>
