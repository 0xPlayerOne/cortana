import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import {
  Combobox,
  ComboboxChip,
  ComboboxChips,
  ComboboxChipsInput,
  ComboboxContent,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from '@/components/shadcn/combobox'
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
} from '@/components/shadcn/pagination'
import { Slider } from '@/components/shadcn/slider'
import { ToggleGroup, ToggleGroupItem } from '@/components/shadcn/toggle-group'

import { AsyncButton } from './async-button'
import { FeedbackState } from './feedback-state'
import { StatusBadge } from './status-badge'
import { ValidatedInput } from './validated-input'

afterEach(() => cleanup())

test('announces status and exposes a consistent busy button contract', () => {
  render(
    <>
      <StatusBadge tone="warning">Sync degraded</StatusBadge>
      <AsyncButton busy busyLabel="Saving source">
        Save source
      </AsyncButton>
    </>
  )

  expect(screen.getByRole('status').textContent).toContain('Sync degraded')
  const button = screen.getByRole('button', { name: 'Saving source' })
  expect(button.getAttribute('aria-busy')).toBe('true')
  expect((button as HTMLButtonElement).disabled).toBe(true)
})

test('announces busy status semantics', () => {
  render(<StatusBadge tone="busy">Indexing</StatusBadge>)
  const status = screen.getByRole('status')
  expect(status.getAttribute('aria-busy')).toBe('true')
  expect(status.textContent).toContain('Indexing')
})

test('keeps error recovery keyboard-operable', async () => {
  const retry = mock(() => undefined)
  const user = userEvent.setup()
  render(
    <FeedbackState
      kind="error"
      title="Index unavailable"
      description="The last bounded request failed."
      onRetry={retry}
    />
  )

  const button = screen.getByRole('button', { name: 'Retry' })
  await user.tab()
  await user.keyboard('{Enter}')
  expect(document.activeElement).toBe(button)
  expect(retry).toHaveBeenCalledTimes(1)
})

test('labels loading and empty feedback without browser-native chrome', () => {
  const { rerender } = render(
    <FeedbackState kind="loading" title="Loading evidence" description="Preparing results." />
  )
  expect(
    screen
      .getByRole('status', { name: 'Loading evidence: Preparing results.' })
      .getAttribute('aria-busy')
  ).toBe('true')

  rerender(<FeedbackState kind="empty" title="No evidence" description="Try another query." />)
  expect(screen.getByText('No evidence')).toBeTruthy()
  expect(screen.getByText('Try another query.')).toBeTruthy()
})

test('associates field help and validation errors programmatically', () => {
  render(
    <ValidatedInput
      id="provider-url"
      label="Provider URL"
      description="Use an approved HTTPS endpoint."
      error="Enter a valid HTTPS URL."
      defaultValue="http://example.test"
    />
  )

  const input = screen.getByRole('textbox', { name: 'Provider URL' })
  expect(input.getAttribute('aria-invalid')).toBe('true')
  expect(input.getAttribute('aria-describedby')).toBe('provider-url-description provider-url-error')
  expect(screen.getByRole('alert').textContent).toBe('Enter a valid HTTPS URL.')
})

test('supports keyboard selection through the shared combobox', async () => {
  const user = userEvent.setup()
  render(
    <Combobox items={['Personal', 'Work']}>
      <ComboboxInput aria-label="Workspace scope" />
      <ComboboxContent>
        <ComboboxList>
          <ComboboxItem value="Personal">Personal</ComboboxItem>
          <ComboboxItem value="Work">Work</ComboboxItem>
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )

  const input = screen.getByRole('combobox', { name: 'Workspace scope' })
  expect(screen.getByRole('button', { name: 'Toggle options' })).toBeTruthy()
  await user.click(input)
  await user.keyboard('{ArrowDown}{Enter}')
  expect((input as HTMLInputElement).value).toBe('Personal')
})

test('names combobox clear and chip-removal actions', () => {
  render(
    <Combobox items={['Personal', 'Work']} defaultValue="Personal">
      <ComboboxInput aria-label="Workspace scope" showClear />
    </Combobox>
  )
  expect(screen.getByRole('button', { name: 'Clear selection' })).toBeTruthy()

  render(
    <Combobox multiple defaultValue={['Personal']}>
      <ComboboxChips>
        <ComboboxChip>Personal</ComboboxChip>
        <ComboboxChipsInput aria-label="Workspace scopes" />
      </ComboboxChips>
    </Combobox>
  )
  expect(screen.getByRole('button', { name: 'Remove item' })).toBeTruthy()
})

test('renders a scalar slider value with exactly one thumb', async () => {
  const { container } = render(<Slider aria-label="Relevance" defaultValue={50} />)
  await waitFor(() => {
    expect(container.querySelectorAll('[data-slot="slider-thumb"]')).toHaveLength(1)
    expect(container.querySelector('input[type="range"]')?.getAttribute('value')).toBe('50')
  })
})

test('honors vertical toggle-group keyboard orientation', async () => {
  const user = userEvent.setup()
  render(
    <ToggleGroup orientation="vertical">
      <ToggleGroupItem value="recent">Recent</ToggleGroupItem>
      <ToggleGroupItem value="relevant">Relevant</ToggleGroupItem>
    </ToggleGroup>
  )

  const recent = screen.getByRole('button', { name: 'Recent' })
  const relevant = screen.getByRole('button', { name: 'Relevant' })
  recent.focus()
  await user.keyboard('{ArrowDown}')
  expect(document.activeElement).toBe(relevant)
})

test('keeps pagination links exposed as links', () => {
  render(
    <Pagination>
      <PaginationContent>
        <PaginationItem>
          <PaginationLink href="/?page=2">2</PaginationLink>
        </PaginationItem>
      </PaginationContent>
    </Pagination>
  )

  const link = screen.getByRole('link', { name: '2' })
  expect(link.getAttribute('href')).toBe('/?page=2')
  expect(link.getAttribute('role')).toBeNull()
  expect(screen.queryByRole('button', { name: '2' })).toBeNull()
})
