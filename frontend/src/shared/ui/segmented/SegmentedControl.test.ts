import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import SegmentedControl from './SegmentedControl.vue'

const OPTIONS = [
  { value: '1h', label: '1h' },
  { value: '24h', label: '24h', title: 'Last 24 hours' },
  { value: '7d', label: '7d' },
] as const

describe('a segmented control', () => {
  it('chooses by click and with the arrow keys, round the ends', async () => {
    const wrapper = mount(SegmentedControl, {
      props: {
        options: OPTIONS,
        label: 'Time range',
        modelValue: '24h',
        'onUpdate:modelValue': (value: string) => wrapper.setProps({ modelValue: value }),
      },
    })
    const radios = () => wrapper.findAll('[role="radio"]')
    expect(wrapper.find('[role="radiogroup"]').attributes('aria-label')).toBe('Time range')
    expect(radios()[1]!.attributes('aria-checked')).toBe('true')
    expect(radios()[1]!.attributes('tabindex')).toBe('0')
    expect(radios()[1]!.attributes('title')).toBe('Last 24 hours')

    await radios()[0]!.trigger('click')
    expect(wrapper.props('modelValue')).toBe('1h')
    await radios()[0]!.trigger('keydown', { key: 'ArrowLeft' })
    expect(wrapper.props('modelValue')).toBe('7d')
    await radios()[2]!.trigger('keydown', { key: 'ArrowDown' })
    expect(wrapper.props('modelValue')).toBe('1h')
    await radios()[0]!.trigger('keydown', { key: 'ArrowRight' })
    await radios()[1]!.trigger('keydown', { key: 'ArrowUp' })
    expect(wrapper.props('modelValue')).toBe('1h')
  })
})
