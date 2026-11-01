import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SidebarRail, type SidebarRailProps } from './SidebarRail'

function setup(props: Partial<SidebarRailProps> = {}) {
  const all: SidebarRailProps = {
    surface: 'chat', section: 'home', newDisabled: false, hasProject: true, queued: 0, profileName: 'Djamal',
    onToggle: vi.fn(), onNew: vi.fn(), onSearch: vi.fn(), onHistory: vi.fn(), onProjects: vi.fn(), onTeam: vi.fn(), onTasks: vi.fn(), onSettings: vi.fn(), onProfile: vi.fn(),
    ...props,
  }
  render(<SidebarRail {...all} />)
  return all
}

describe('SidebarRail', () => {
  it('shows the chat items with tooltips and navigates on click', () => {
    const props = setup()
    expect(screen.getByRole('navigation', { name: 'Collapsed sidebar' })).toBeTruthy()
    const projects = screen.getByRole('button', { name: 'Projects' })
    expect(projects.getAttribute('data-tip')).toBe('Projects')
    fireEvent.click(projects)
    expect(props.onProjects).toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Open sidebar' }))
    expect(props.onToggle).toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'New chat' }))
    expect(props.onNew).toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Profile: Djamal' }).textContent).toBe('D')
  })

  it('marks the active place and uses Sessions without Projects in Code', () => {
    setup({ surface: 'code', section: 'board' })
    expect(screen.queryByRole('button', { name: 'Projects' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Sessions' }).getAttribute('aria-current')).toBe('page')
    expect(screen.getByRole('button', { name: 'Team' }).getAttribute('aria-current')).toBeNull()
  })

  it('disables new session without a project and moves focus with arrow keys', () => {
    setup({ surface: 'code', newDisabled: true, hasProject: false })
    expect((screen.getByRole('button', { name: 'New session' }) as HTMLButtonElement).disabled).toBe(true)
    const toggle = screen.getByRole('button', { name: 'Open sidebar' })
    toggle.focus()
    fireEvent.keyDown(toggle, { key: 'ArrowDown' })
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Team' }))
    fireEvent.keyDown(document.activeElement!, { key: 'End' })
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Profile: Djamal' }))
  })

  it('shows the queued count on Tasks', () => {
    setup({ queued: 3 })
    expect(screen.getByLabelText('3 queued').textContent).toBe('3')
  })
})
