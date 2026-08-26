import type { ComponentProps } from 'react'

import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/shadcn/field'
import { Input } from '@/components/shadcn/input'

type ValidatedInputProps = Omit<ComponentProps<typeof Input>, 'id'> & {
  id: string
  label: string
  description?: string
  error?: string
}

export function ValidatedInput({ id, label, description, error, ...props }: ValidatedInputProps) {
  const descriptionId = description ? `${id}-description` : undefined
  const errorId = error ? `${id}-error` : undefined
  const describedBy = [descriptionId, errorId].filter(Boolean).join(' ') || undefined

  return (
    <Field data-invalid={error ? '' : undefined}>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        {...props}
      />
      {description ? <FieldDescription id={descriptionId}>{description}</FieldDescription> : null}
      {error ? <FieldError id={errorId}>{error}</FieldError> : null}
    </Field>
  )
}
