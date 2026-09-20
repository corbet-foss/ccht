/** Browser behavior of the generic per-step config form (StepConfig.svelte).
 *
 * Covers step rendering, backend/model selection callbacks, the account
 * slot branch, extra select/boolean controls, refresh, and error display.
 */
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import { afterEach, describe, expect, test, vi } from 'vitest';
import { createRawSnippet } from 'svelte';
import StepConfig from './StepConfig.svelte';
import type { StepData } from './StepConfig.svelte';

afterEach(() => cleanup());

function step(overrides: Partial<StepData> = {}): StepData {
  return {
    id: 'creator',
    label: 'Creator chat',
    backendValue: '',
    backendOptions: [{ value: 'local', name: 'Local' }],
    backendPlaceholder: 'Configured default',
    accountConnected: true,
    accountBusy: false,
    configurationError: null,
    modelValue: 'qwen',
    modelOptions: [{ value: 'qwen', name: 'Qwen' }],
    refreshable: true,
    extraOptions: [],
    extraValues: {},
    ...overrides,
  };
}

function openSteps(steps: StepData[], props: Record<string, unknown> = {}) {
  return render(StepConfig, {
    steps,
    onSelectBackend: () => {},
    onSelectModel: () => {},
    onSelectOption: () => {},
    onRefresh: () => {},
    ...props,
  });
}

describe('step rendering', () => {
  test('renders one labelled group per step with backend and model selects', () => {
    const { getByRole, getByLabelText } = openSteps([step(), step({ id: 'compile', label: 'Proposal compilation' })]);
    expect(getByRole('group', { name: 'Creator chat' })).toBeTruthy();
    expect(getByRole('group', { name: 'Proposal compilation' })).toBeTruthy();
    expect(getByLabelText('Creator chat assistant')).toBeTruthy();
    expect(getByLabelText('Creator chat model')).toBeTruthy();
  });

  test('backend and model selections forward step id and value', async () => {
    const onSelectBackend = vi.fn();
    const onSelectModel = vi.fn();
    const { getByLabelText } = openSteps([step()], { onSelectBackend, onSelectModel });
    await fireEvent.change(getByLabelText('Creator chat assistant'), { target: { value: 'local' } });
    expect(onSelectBackend).toHaveBeenCalledWith('creator', 'local');
    await fireEvent.change(getByLabelText('Creator chat model'), { target: { value: 'qwen' } });
    expect(onSelectModel).toHaveBeenCalledWith('creator', 'qwen');
  });

  test('empty model options render a No model placeholder', () => {
    const { getByLabelText } = openSteps([step({ modelValue: '', modelOptions: [] })]);
    expect(getByLabelText('Creator chat model').textContent).toContain('No model');
  });
});

describe('account branch', () => {
  test('disconnected steps without a slot fall back to model controls', () => {
    const { getByLabelText } = openSteps([step({ accountConnected: false })]);
    expect(getByLabelText('Creator chat model')).toBeTruthy();
  });

  test('disconnected steps render the account slot with the step id', () => {
    const accountSlot = createRawSnippet<{ step: string }>((getStep) => ({
      render: () => `<button type="button" data-step="${getStep().step}">connect</button>`,
    }));
    const { getByRole, queryByLabelText } = openSteps([step({ accountConnected: false })], { accountSlot });
    expect(queryByLabelText('Creator chat model')).toBeNull();
    expect(getByRole('button', { name: 'connect' }).getAttribute('data-step')).toBe('creator');
  });

  test('connected steps render model controls even with a slot provided', () => {
    const { getByLabelText } = render(StepConfig, {
      steps: [step({ accountConnected: true })],
      onSelectBackend: () => {},
      onSelectModel: () => {},
      onSelectOption: () => {},
      onRefresh: () => {},
      accountSlot: (() => {}) as never,
    });
    expect(getByLabelText('Creator chat model')).toBeTruthy();
  });
});

describe('extra controls', () => {
  const options = [
    {
      id: 'model',
      name: 'Model',
      category: 'model',
      type: 'select',
      currentValue: 'one',
      options: [{ value: 'one', name: 'One' }],
    },
    {
      id: 'thinking',
      name: 'Thinking',
      type: 'boolean',
      currentValue: true,
    },
  ];

  test('model-category controls are filtered out; others render', () => {
    const { queryByLabelText, getByLabelText } = openSteps([step({ extraOptions: options as never })]);
    expect(queryByLabelText('Model')).toBeNull();
    expect(getByLabelText('Thinking')).toBeTruthy();
  });

  test('select and boolean edits forward option and typed value', async () => {
    const onSelectOption = vi.fn();
    const selectOptions = [
      {
        id: 'effort',
        name: 'Effort',
        type: 'select',
        currentValue: 'low',
        options: [{ value: 'low', name: 'Low' }, { value: 'high', name: 'High' }],
      },
    ];
    const { getByLabelText } = openSteps(
      [step({ extraOptions: [...selectOptions, options[1]] as never, extraValues: { effort: 'low', thinking: true } })],
      { onSelectOption },
    );
    await fireEvent.change(getByLabelText('Effort'), { target: { value: 'high' } });
    expect(onSelectOption).toHaveBeenCalledWith('creator', expect.objectContaining({ id: 'effort' }), 'high');
    const thinking = getByLabelText('Thinking') as HTMLInputElement;
    expect(thinking.checked).toBe(true);
    await fireEvent.click(thinking);
    expect(onSelectOption).toHaveBeenCalledWith('creator', expect.objectContaining({ id: 'thinking' }), false);
  });

  test('grouped select choices flatten in order', async () => {
    const onSelectOption = vi.fn();
    const { getByLabelText } = openSteps(
      [
        step({
          extraOptions: [
            {
              id: 'model2',
              name: 'Second',
              type: 'select',
              currentValue: 'b',
              options: [{ group: 'g', name: 'G', options: [{ value: 'a', name: 'A' }, { value: 'b', name: 'B' }] }],
            },
          ] as never,
        }),
      ],
      { onSelectOption },
    );
    const select = getByLabelText('Second') as HTMLSelectElement;
    expect([...select.options].map((option) => option.value)).toEqual(['a', 'b']);
  });
});

describe('refresh and errors', () => {
  test('refresh button forwards the step id and reflects loading', async () => {
    const onRefresh = vi.fn();
    const idle = openSteps([step()], { onRefresh });
    await fireEvent.click(idle.getByRole('button', { name: 'Refresh models' }));
    expect(onRefresh).toHaveBeenCalledWith('creator');
    const loading = openSteps([step()], { onRefresh, refreshing: true });
    expect(loading.getByRole('button', { name: 'Loading controls…' })).toBeTruthy();
  });

  test('configuration errors render as alerts', () => {
    const { getByText } = openSteps([step({ configurationError: 'stale options' })]);
    expect(getByText('stale options')).toBeTruthy();
  });

  test('no refresh row while disconnected or not refreshable', () => {
    const { queryByRole, rerender } = openSteps([step({ accountConnected: false })]);
    expect(queryByRole('button', { name: 'Refresh models' })).toBeNull();
  });

  test('non-refreshable steps omit refresh even when connected', () => {
    const { queryByRole } = openSteps([step({ refreshable: false })]);
    expect(queryByRole('button', { name: /Refresh models|Loading/ })).toBeNull();
  });
});
