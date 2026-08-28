import type { ChangeEventHandler, ComponentProps } from 'react'

import { InputGroup, InputGroupButton, InputGroupInput } from '../shadcn/input-group'

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
