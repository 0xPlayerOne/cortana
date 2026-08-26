import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'

import { M7ShadcnPrototype } from './M7ShadcnPrototype'

afterEach(() => cleanup())

test('switches evidence tabs without losing the document workflow', async () => {
  render(<M7ShadcnPrototype />)

  expect(screen.getByRole('heading', { name: 'Release evidence' })).toBeTruthy()
  fireEvent.click(screen.getByRole('tab', { name: 'Answer' }))
  expect(
    await screen.findByText(
      'Synthesized answers will compose the same evidence cards and citations.'
    )
  ).toBeTruthy()
  fireEvent.click(screen.getByRole('tab', { name: 'Document' }))
  expect(await screen.findByText('Evidence spine')).toBeTruthy()
})
