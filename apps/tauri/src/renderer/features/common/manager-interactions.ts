import type { SyntheticEvent } from 'react'

const nestedManagerControlSelector = [
  'button',
  'input',
  'textarea',
  'select',
  'a',
  '[role="button"]',
  '[role="menuitem"]',
  '[contenteditable="true"]'
].join(',')

/**
 * Manager rows are keyboard-focusable and own click/double-click shortcuts.
 * Ignore an event when it originated from a nested control so an edit button
 * cannot also trigger the row's open/connect action while bubbling.
 */
export function targetsNestedManagerControl(event: SyntheticEvent<HTMLElement>) {
  const target = event.target
  if (!(target instanceof Element)) {
    return false
  }

  const control = target.closest(nestedManagerControlSelector)
  return control !== null && control !== event.currentTarget
}
