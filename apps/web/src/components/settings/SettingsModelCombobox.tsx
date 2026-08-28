import type { ComponentProps, ReactNode } from 'react'

import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from '../shadcn/combobox'
import { useSettingsRenderer } from './SettingsSurface'

export function SettingsModelCombobox({
  value,
  choices,
  onValueChange,
  ...props
}: Omit<ComponentProps<'input'>, 'value' | 'onChange'> & {
  value: string
  choices: Array<{ value: string; label: ReactNode }>
  onValueChange: (value: string) => void
}) {
  const renderer = useSettingsRenderer()
  if (renderer === 'legacy') {
    return (
      <select
        id={props.id}
        aria-label={props['aria-label']}
        aria-describedby={props['aria-describedby']}
        aria-invalid={props['aria-invalid']}
        value={value}
        disabled={props.disabled}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {choices.map((choice) => (
          <option key={choice.value} value={choice.value}>
            {choice.label}
          </option>
        ))}
      </select>
    )
  }
  return (
    <Combobox value={value} onValueChange={(next) => next && onValueChange(String(next))}>
      <ComboboxInput {...props} value={value} />
      <ComboboxContent>
        <ComboboxEmpty>No matching models.</ComboboxEmpty>
        <ComboboxList>
          {choices.map((choice) => (
            <ComboboxItem key={choice.value} value={choice.value}>
              {choice.label}
            </ComboboxItem>
          ))}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )
}
