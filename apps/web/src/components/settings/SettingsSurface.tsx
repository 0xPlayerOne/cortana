import {
  Children,
  type ComponentProps,
  type ChangeEvent,
  isValidElement,
  type ReactNode,
} from 'react'

import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from '../shadcn/accordion'
import { Alert } from '../shadcn/alert'
import { TooltipButton as Button } from '../cortana/TooltipButton'
import { Card } from '../shadcn/card'
import { Checkbox } from '../shadcn/checkbox'
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from '../shadcn/field'
import { Input } from '../shadcn/input'
import { RadioGroup, RadioGroupItem } from '../shadcn/radio-group'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../shadcn/select'
import { Switch } from '../shadcn/switch'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../shadcn/tabs'
import { Textarea } from '../shadcn/textarea'

export function SettingsSurfaceProvider({ children }: { children: ReactNode }) {
  return children
}

type SettingsButtonProps = Omit<ComponentProps<typeof Button>, 'variant' | 'size'> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'ghost' | 'icon' | 'compact'
}

export function SettingsButton({ variant = 'secondary', ...props }: SettingsButtonProps) {
  return (
    <Button
      {...props}
      variant={
        variant === 'primary'
          ? 'default'
          : variant === 'danger'
            ? 'destructive'
            : variant === 'ghost' || variant === 'icon'
              ? 'ghost'
              : 'secondary'
      }
      size={variant === 'icon' ? 'icon' : variant === 'compact' ? 'sm' : 'default'}
    />
  )
}

export function SettingsInput(props: ComponentProps<'input'>) {
  return <Input {...props} />
}

export function SettingsTextarea(props: ComponentProps<'textarea'>) {
  return <Textarea {...props} />
}

export function SettingsCard(props: ComponentProps<'div'>) {
  return <Card {...props} />
}

export function SettingsAlert({
  variant = 'default',
  ...props
}: ComponentProps<'div'> & { variant?: 'default' | 'destructive' }) {
  return <Alert variant={variant} {...props} />
}

export function SettingsField(props: ComponentProps<'div'>) {
  return <Field {...props} />
}

export function SettingsFieldGroup(props: ComponentProps<'div'>) {
  return <FieldGroup {...props} />
}

export function SettingsFieldSet(props: ComponentProps<'fieldset'>) {
  return <FieldSet {...props} />
}

export function SettingsFieldLegend(props: ComponentProps<'legend'>) {
  return <FieldLegend {...props} />
}

export function SettingsFieldLabel(props: ComponentProps<'label'>) {
  return <FieldLabel {...props} />
}

export function SettingsFieldDescription(props: ComponentProps<'p'>) {
  return <FieldDescription {...props} />
}

export function SettingsFieldError(props: ComponentProps<'div'>) {
  return <FieldError {...props} />
}

export function SettingsCheckbox({ onChange, ...props }: Omit<ComponentProps<'input'>, 'type'>) {
  return (
    <Checkbox
      id={props.id}
      name={props.name}
      checked={Boolean(props.checked)}
      disabled={props.disabled}
      required={props.required}
      aria-label={props['aria-label']}
      aria-describedby={props['aria-describedby']}
      aria-invalid={props['aria-invalid']}
      title={props.title}
      onCheckedChange={(checked) => {
        onChange?.({
          target: { checked },
          currentTarget: { checked },
        } as unknown as ChangeEvent<HTMLInputElement>)
      }}
    />
  )
}

export function SettingsSwitch({ onChange, ...props }: Omit<ComponentProps<'input'>, 'type'>) {
  return (
    <Switch
      id={props.id}
      name={props.name}
      checked={Boolean(props.checked)}
      disabled={props.disabled}
      required={props.required}
      aria-label={props['aria-label']}
      aria-describedby={props['aria-describedby']}
      aria-invalid={props['aria-invalid']}
      title={props.title}
      onCheckedChange={(checked) => {
        onChange?.({
          target: { checked },
          currentTarget: { checked },
        } as unknown as ChangeEvent<HTMLInputElement>)
      }}
    />
  )
}

export function SettingsRadioGroup({
  value,
  onValueChange,
  children,
  ...props
}: ComponentProps<'div'> & {
  value: string
  onValueChange: (value: string) => void
}) {
  return (
    <RadioGroup value={value} onValueChange={onValueChange} {...props}>
      {children}
    </RadioGroup>
  )
}

export function SettingsRadio({ value, ...props }: Omit<ComponentProps<'input'>, 'type'>) {
  return (
    <RadioGroupItem
      value={String(value ?? '')}
      disabled={props.disabled}
      aria-label={props['aria-label']}
      aria-describedby={props['aria-describedby']}
    />
  )
}

export function SettingsTabs({
  value,
  onValueChange,
  children,
  ...props
}: ComponentProps<'div'> & { value: string; onValueChange: (value: string) => void }) {
  return (
    <Tabs value={value} onValueChange={onValueChange} {...props}>
      {children}
    </Tabs>
  )
}

export function SettingsTabsList({
  variant,
  ...props
}: ComponentProps<'div'> & { variant?: 'default' | 'line' }) {
  return <TabsList variant={variant} {...props} />
}

export function SettingsTabsTrigger({
  value,
  ...props
}: ComponentProps<'button'> & { value: string }) {
  return <TabsTrigger value={value} {...props} />
}

export function SettingsTabsContent({
  value,
  ...props
}: ComponentProps<'div'> & { value: string }) {
  return <TabsContent value={value} {...props} />
}

export function SettingsAccordion({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <Accordion className={className}>{children}</Accordion>
}

export function SettingsAccordionItem({
  value,
  className,
  children,
}: {
  value: string
  className?: string
  children: ReactNode
}) {
  return (
    <AccordionItem value={value} className={className}>
      {children}
    </AccordionItem>
  )
}

export function SettingsAccordionTrigger({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <AccordionTrigger className={className}>{children}</AccordionTrigger>
}

export function SettingsAccordionContent({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <AccordionContent className={className}>{children}</AccordionContent>
}

type SelectOption = {
  value: string
  label: ReactNode
  disabled: boolean
}

function selectOptions(children: ReactNode): SelectOption[] {
  return Children.toArray(children).flatMap((child) => {
    if (
      !isValidElement<{ value?: string | number; disabled?: boolean; children?: ReactNode }>(child)
    ) {
      return []
    }
    if (child.type !== 'option') return selectOptions(child.props.children)
    return [
      {
        value: String(child.props.value ?? ''),
        label: child.props.children,
        disabled: Boolean(child.props.disabled),
      },
    ]
  })
}

export function SettingsSelect({ children, onChange, value, ...props }: ComponentProps<'select'>) {
  const options = selectOptions(children)
  return (
    <Select
      value={String(value ?? '')}
      disabled={props.disabled}
      name={props.name}
      required={props.required}
      onValueChange={(next) =>
        onChange?.({
          target: { value: next },
          currentTarget: { value: next },
        } as unknown as ChangeEvent<HTMLSelectElement>)
      }
    >
      <SelectTrigger
        id={props.id}
        className={['w-full border-border bg-background shadow-xs', props.className]
          .filter(Boolean)
          .join(' ')}
        aria-label={props['aria-label']}
        aria-describedby={props['aria-describedby']}
        aria-invalid={props['aria-invalid']}
        title={props.title}
        style={props.style}
      >
        <SelectValue>
          {(selected) =>
            options.find((option) => option.value === String(selected))?.label ?? selected
          }
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
