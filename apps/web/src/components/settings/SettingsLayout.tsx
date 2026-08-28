import {
  Children,
  cloneElement,
  isValidElement,
  type ReactNode,
  useEffect,
  useId,
  useState,
} from 'react'

import type { DesktopSettings } from '../../types'
import {
  SettingsCard,
  SettingsCheckbox,
  SettingsFieldDescription,
  SettingsFieldError,
  SettingsFieldLegend,
  SettingsFieldLabel,
  SettingsField,
  SettingsFieldSet,
  SettingsInput as Input,
  SettingsRadio,
  SettingsSelect as Select,
  SettingsTextarea as Textarea,
} from './SettingsSurface'

export type SettingsSectionProps = {
  settings: DesktopSettings
  update: (change: (draft: DesktopSettings) => DesktopSettings) => void
}

export function SettingsSection({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section className="settings-section">
      <SettingsCard className="settings-section-card">
        <h2>{title}</h2>
        <p>{description}</p>
        {children}
      </SettingsCard>
    </section>
  )
}

export function Field({
  label,
  hint,
  error,
  wide = false,
  group = false,
  controlId: providedControlId,
  children,
}: {
  label: string
  hint?: string
  error?: string
  wide?: boolean
  group?: boolean
  controlId?: string
  children: ReactNode
}) {
  const generatedControlId = useId()
  const controlId = providedControlId ?? generatedControlId
  const descriptionId = hint ? `${controlId}-description` : undefined
  const errorId = error ? `${controlId}-error` : undefined
  const describedBy = [descriptionId, errorId].filter(Boolean).join(' ') || undefined
  const groupLabelId = `${controlId}-label`

  if (group) {
    return (
      <SettingsFieldSet
        className={`form-field ${wide ? 'wide' : ''}`}
        aria-describedby={describedBy}
      >
        <SettingsFieldLegend className="form-field-label">{label}</SettingsFieldLegend>
        {children}
        {hint && <SettingsFieldDescription id={descriptionId}>{hint}</SettingsFieldDescription>}
        {error && <SettingsFieldError id={errorId}>{error}</SettingsFieldError>}
      </SettingsFieldSet>
    )
  }

  let controlAssigned = Boolean(providedControlId)
  const assignControl = (nodes: ReactNode): ReactNode =>
    Children.map(nodes, (node) => {
      if (
        !isValidElement<{
          id?: string
          'aria-describedby'?: string
          'aria-invalid'?: boolean
          children?: ReactNode
        }>(node)
      )
        return node
      if (
        !controlAssigned &&
        ([Input, Select, Textarea, SettingsCheckbox, SettingsRadio] as unknown[]).includes(
          node.type
        )
      ) {
        controlAssigned = true
        return cloneElement(node, {
          id: controlId,
          'aria-describedby': describedBy,
          'aria-invalid': Boolean(error),
        })
      }
      if (node.props.children) {
        return cloneElement(node, { children: assignControl(node.props.children) })
      }
      return node
    })

  const assignedChildren = assignControl(children)

  return (
    <SettingsField
      className={`form-field ${wide ? 'wide' : ''}`}
      role={controlAssigned ? undefined : 'group'}
      aria-labelledby={controlAssigned ? undefined : groupLabelId}
      aria-describedby={controlAssigned ? undefined : describedBy}
    >
      {controlAssigned ? (
        <SettingsFieldLabel htmlFor={controlId} className="form-field-label">
          {label}
        </SettingsFieldLabel>
      ) : (
        <span id={groupLabelId} className="form-field-label">
          {label}
        </span>
      )}
      {assignedChildren}
      {hint && <SettingsFieldDescription id={descriptionId}>{hint}</SettingsFieldDescription>}
      {error && <SettingsFieldError id={errorId}>{error}</SettingsFieldError>}
    </SettingsField>
  )
}

export function NumberField({
  label,
  hint,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  hint?: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
}) {
  const [draft, setDraft] = useState(String(value))
  const [error, setError] = useState('')

  useEffect(() => {
    setDraft(String(value))
    setError('')
  }, [value])

  const validate = (raw: string) => {
    if (!raw) return `${label} is required.`
    const next = Number(raw)
    if (!Number.isFinite(next) || !Number.isInteger(next)) {
      return `${label} must be a whole number.`
    }
    if (next < min || next > max) {
      return `${label} must be between ${min} and ${max}.`
    }
    return ''
  }

  return (
    <Field label={label} hint={hint} error={error}>
      <Input
        type="number"
        aria-label={label}
        value={draft}
        min={min}
        max={max}
        onChange={(event) => {
          const raw = event.target.value
          setDraft(raw)
          const nextError = validate(raw)
          setError(nextError)
          if (!nextError) onChange(Number(raw))
        }}
        onBlur={() => {
          const nextError = validate(draft)
          if (nextError) {
            setDraft(String(value))
            setError('')
          }
        }}
        required
      />
    </Field>
  )
}
