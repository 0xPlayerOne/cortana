import type { ChangeEventHandler, ComponentProps } from 'react'

import { InputGroup, InputGroupButton, InputGroupInput } from '../shadcn/input-group'
import { useSettingsRenderer } from './SettingsSurface'

export function SettingsSecretInputGroup({
  value,
  disabled,
  onChange,
  onClear,
  ...props
}: {
  value: string
  disabled: boolean
  onChange: ChangeEventHandler<HTMLInputElement>
  onClear?: () => void
} & Pick<ComponentProps<'input'>, 'id' | 'aria-label' | 'aria-describedby' | 'aria-invalid'>) {
  const renderer = useSettingsRenderer()
  if (renderer === 'legacy') {
    return (
      <div className="secret-input">
        <input
          {...props}
          aria-label={props['aria-label'] ?? 'New API key'}
          type="password"
          autoComplete="new-password"
          value={value}
          disabled={disabled}
          onChange={onChange}
        />
        {onClear && (
          <button type="button" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
    )
  }
  return (
    <InputGroup className="secret-input">
      <InputGroupInput
        {...props}
        aria-label={props['aria-label'] ?? 'New API key'}
        type="password"
        autoComplete="new-password"
        value={value}
        disabled={disabled}
        onChange={onChange}
      />
      {onClear && (
        <InputGroupButton size="xs" onClick={onClear}>
          Clear
        </InputGroupButton>
      )}
    </InputGroup>
  )
}
