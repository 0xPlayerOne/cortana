import {
  Children,
  createContext,
  type ComponentProps,
  type ChangeEvent,
  isValidElement,
  type ReactNode,
  useContext,
} from 'react'

import { useM7SurfacePrimitives } from '../m7/M7SurfacePrimitives'
import { Button as LegacyButton, type ButtonProps } from '../ui/Button'

type SettingsRenderer = 'legacy' | 'shadcn'

const SettingsRendererContext = createContext<SettingsRenderer>('legacy')
const SettingsTabsContext = createContext<{
  value: string
  onValueChange: (value: string) => void
} | null>(null)

export function SettingsSurfaceProvider({
  renderer,
  children,
}: {
  renderer: SettingsRenderer
  children: ReactNode
}) {
  const primitives = useM7SurfacePrimitives()
  if (renderer === 'shadcn' && !primitives) {
    throw new Error('The shadcn settings renderer requires M7SurfacePrimitivesProvider.')
  }
  return (
    <SettingsRendererContext.Provider value={renderer}>{children}</SettingsRendererContext.Provider>
  )
}

export function SettingsButton({ variant = 'secondary', ...props }: ButtonProps) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnButton = useM7SurfacePrimitives()?.Button
  if (renderer === 'legacy' || !ShadcnButton) {
    return <LegacyButton variant={variant} {...props} />
  }
  return (
    <ShadcnButton
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
  const renderer = useContext(SettingsRendererContext)
  const ShadcnInput = useM7SurfacePrimitives()?.Input
  return renderer === 'shadcn' && ShadcnInput ? <ShadcnInput {...props} /> : <input {...props} />
}

// eslint-disable-next-line react-refresh/only-export-components
export function useSettingsRenderer() {
  return useContext(SettingsRendererContext)
}

export function SettingsTextarea(props: ComponentProps<'textarea'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnTextarea = useM7SurfacePrimitives()?.Textarea
  return renderer === 'shadcn' && ShadcnTextarea ? (
    <ShadcnTextarea {...props} />
  ) : (
    <textarea {...props} />
  )
}

export function SettingsCard(props: ComponentProps<'div'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnCard = useM7SurfacePrimitives()?.Card
  return renderer === 'shadcn' && ShadcnCard ? <ShadcnCard {...props} /> : <div {...props} />
}

export function SettingsAlert({
  variant = 'default',
  ...props
}: ComponentProps<'div'> & { variant?: 'default' | 'destructive' }) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnAlert = useM7SurfacePrimitives()?.Alert
  return renderer === 'shadcn' && ShadcnAlert ? (
    <ShadcnAlert variant={variant} {...props} />
  ) : (
    <div {...props} />
  )
}

export function SettingsField({ children, ...props }: ComponentProps<'div'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnField = useM7SurfacePrimitives()?.Field
  return renderer === 'shadcn' && ShadcnField ? (
    <ShadcnField {...props}>{children}</ShadcnField>
  ) : (
    <div {...props}>{children}</div>
  )
}

export function SettingsFieldGroup(props: ComponentProps<'div'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldGroup = useM7SurfacePrimitives()?.FieldGroup
  return renderer === 'shadcn' && ShadcnFieldGroup ? (
    <ShadcnFieldGroup {...props} />
  ) : (
    <div {...props} />
  )
}

export function SettingsFieldSet(props: ComponentProps<'fieldset'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldSet = useM7SurfacePrimitives()?.FieldSet
  return renderer === 'shadcn' && ShadcnFieldSet ? (
    <ShadcnFieldSet {...props} />
  ) : (
    <fieldset {...props} />
  )
}

export function SettingsFieldLegend(props: ComponentProps<'legend'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldLegend = useM7SurfacePrimitives()?.FieldLegend
  return renderer === 'shadcn' && ShadcnFieldLegend ? (
    <ShadcnFieldLegend {...props} />
  ) : (
    <legend {...props} />
  )
}

export function SettingsFieldLabel(props: ComponentProps<'label'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldLabel = useM7SurfacePrimitives()?.FieldLabel
  return renderer === 'shadcn' && ShadcnFieldLabel ? (
    <ShadcnFieldLabel {...props} />
  ) : (
    <label {...props} />
  )
}

export function SettingsFieldDescription(props: ComponentProps<'p'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldDescription = useM7SurfacePrimitives()?.FieldDescription
  return renderer === 'shadcn' && ShadcnFieldDescription ? (
    <ShadcnFieldDescription {...props} />
  ) : (
    <small {...props} />
  )
}

export function SettingsFieldError(props: ComponentProps<'div'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnFieldError = useM7SurfacePrimitives()?.FieldError
  return renderer === 'shadcn' && ShadcnFieldError ? (
    <ShadcnFieldError {...props} />
  ) : (
    <div role="alert" {...props} />
  )
}

export function SettingsCheckbox({ onChange, ...props }: Omit<ComponentProps<'input'>, 'type'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnCheckbox = useM7SurfacePrimitives()?.Checkbox
  if (renderer === 'legacy' || !ShadcnCheckbox) {
    return <input type="checkbox" onChange={onChange} {...props} />
  }
  return (
    <ShadcnCheckbox
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
  const renderer = useContext(SettingsRendererContext)
  const ShadcnSwitch = useM7SurfacePrimitives()?.Switch
  if (renderer === 'legacy' || !ShadcnSwitch) {
    return <input type="checkbox" onChange={onChange} {...props} />
  }
  return (
    <ShadcnSwitch
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
  const renderer = useContext(SettingsRendererContext)
  const ShadcnRadioGroup = useM7SurfacePrimitives()?.RadioGroup
  if (renderer === 'legacy' || !ShadcnRadioGroup) {
    return <div {...props}>{children}</div>
  }
  return (
    <ShadcnRadioGroup value={value} onValueChange={onValueChange} {...props}>
      {children}
    </ShadcnRadioGroup>
  )
}

export function SettingsRadio({
  value,
  checked,
  onChange,
  ...props
}: Omit<ComponentProps<'input'>, 'type'>) {
  const renderer = useContext(SettingsRendererContext)
  const ShadcnRadioGroupItem = useM7SurfacePrimitives()?.RadioGroupItem
  if (renderer === 'legacy' || !ShadcnRadioGroupItem) {
    return <input type="radio" value={value} checked={checked} onChange={onChange} {...props} />
  }
  return (
    <ShadcnRadioGroupItem
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
  const renderer = useContext(SettingsRendererContext)
  const Tabs = useM7SurfacePrimitives()?.Tabs
  const content =
    renderer === 'shadcn' && Tabs ? (
      <Tabs value={value} onValueChange={onValueChange} {...props}>
        {children}
      </Tabs>
    ) : (
      <div {...props}>{children}</div>
    )
  return (
    <SettingsTabsContext.Provider value={{ value, onValueChange }}>
      {content}
    </SettingsTabsContext.Provider>
  )
}

export function SettingsTabsList({
  variant,
  ...props
}: ComponentProps<'div'> & { variant?: 'default' | 'line' }) {
  const renderer = useContext(SettingsRendererContext)
  const TabsList = useM7SurfacePrimitives()?.TabsList
  return renderer === 'shadcn' && TabsList ? (
    <TabsList variant={variant} {...props} />
  ) : (
    <div role="tablist" {...props} />
  )
}

export function SettingsTabsTrigger({
  value,
  ...props
}: ComponentProps<'button'> & { value: string }) {
  const renderer = useContext(SettingsRendererContext)
  const tabs = useContext(SettingsTabsContext)
  const TabsTrigger = useM7SurfacePrimitives()?.TabsTrigger
  return renderer === 'shadcn' && TabsTrigger ? (
    <TabsTrigger value={value} {...props} />
  ) : (
    <button
      type="button"
      role="tab"
      aria-selected={tabs?.value === value}
      {...props}
      onClick={(event) => {
        props.onClick?.(event)
        if (!event.defaultPrevented) tabs?.onValueChange(value)
      }}
    />
  )
}

export function SettingsTabsContent({
  value,
  ...props
}: ComponentProps<'div'> & { value: string }) {
  const renderer = useContext(SettingsRendererContext)
  const TabsContent = useM7SurfacePrimitives()?.TabsContent
  return renderer === 'shadcn' && TabsContent ? (
    <TabsContent value={value} {...props} />
  ) : (
    <div role="tabpanel" {...props} />
  )
}

export function SettingsAccordion({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  const renderer = useContext(SettingsRendererContext)
  const Accordion = useM7SurfacePrimitives()?.Accordion
  return renderer === 'shadcn' && Accordion ? (
    <Accordion className={className}>{children}</Accordion>
  ) : (
    <div className={className}>{children}</div>
  )
}

export function SettingsAccordionItem({
  value,
  ...props
}: {
  value: string
  className?: string
  children: ReactNode
}) {
  const renderer = useContext(SettingsRendererContext)
  const AccordionItem = useM7SurfacePrimitives()?.AccordionItem
  return renderer === 'shadcn' && AccordionItem ? (
    <AccordionItem value={value} className={props.className}>
      {props.children}
    </AccordionItem>
  ) : (
    <details {...props} />
  )
}

export function SettingsAccordionTrigger({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  const renderer = useContext(SettingsRendererContext)
  const AccordionTrigger = useM7SurfacePrimitives()?.AccordionTrigger
  return renderer === 'shadcn' && AccordionTrigger ? (
    <AccordionTrigger className={className}>{children}</AccordionTrigger>
  ) : (
    <summary className={className}>{children}</summary>
  )
}

export function SettingsAccordionContent({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  const renderer = useContext(SettingsRendererContext)
  const AccordionContent = useM7SurfacePrimitives()?.AccordionContent
  return renderer === 'shadcn' && AccordionContent ? (
    <AccordionContent className={className}>{children}</AccordionContent>
  ) : (
    <div className={className}>{children}</div>
  )
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
  const renderer = useContext(SettingsRendererContext)
  const primitives = useM7SurfacePrimitives()
  const ShadcnSelect = primitives?.Select
  const ShadcnSelectContent = primitives?.SelectContent
  const ShadcnSelectItem = primitives?.SelectItem
  const ShadcnSelectTrigger = primitives?.SelectTrigger
  const ShadcnSelectValue = primitives?.SelectValue
  if (
    renderer === 'legacy' ||
    !ShadcnSelect ||
    !ShadcnSelectContent ||
    !ShadcnSelectItem ||
    !ShadcnSelectTrigger ||
    !ShadcnSelectValue
  ) {
    return (
      <select value={value} onChange={onChange} {...props}>
        {children}
      </select>
    )
  }
  return (
    <ShadcnSelect
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
      <ShadcnSelectTrigger
        id={props.id}
        className={props.className}
        aria-label={props['aria-label']}
        aria-describedby={props['aria-describedby']}
        aria-invalid={props['aria-invalid']}
        title={props.title}
        style={props.style}
      >
        <ShadcnSelectValue />
      </ShadcnSelectTrigger>
      <ShadcnSelectContent>
        {selectOptions(children).map((option) => (
          <ShadcnSelectItem key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </ShadcnSelectItem>
        ))}
      </ShadcnSelectContent>
    </ShadcnSelect>
  )
}
