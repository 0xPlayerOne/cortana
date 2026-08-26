import { afterEach, expect, mock, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'

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

test('keeps error recovery keyboard-operable', () => {
  const retry = mock(() => undefined)
  render(
    <FeedbackState
      kind="error"
      title="Index unavailable"
      description="The last bounded request failed."
      onRetry={retry}
    />
  )

  const button = screen.getByRole('button', { name: 'Retry' })
  button.focus()
  fireEvent.keyDown(button, { key: 'Enter' })
  fireEvent.click(button)
  expect(document.activeElement).toBe(button)
  expect(retry).toHaveBeenCalledTimes(1)
})

test('labels loading and empty feedback without browser-native chrome', () => {
  const { rerender } = render(
    <FeedbackState kind="loading" title="Loading evidence" description="Preparing results." />
  )
  expect(screen.getByLabelText('Loading evidence').getAttribute('aria-busy')).toBe('true')

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
