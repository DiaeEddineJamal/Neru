import type { TourStep } from './Tour'

const mod = /Mac/i.test(navigator.platform) ? '⌘' : 'Ctrl'

/** The first-run walkthrough of the whole app. Steps for controls that are not on screen
 * (no project open, chat surface) are skipped, so it fits whatever the user has set up. */
export const APP_TOUR: TourStep[] = [
  { title: 'Welcome to Neru', body: <><p>A two-minute look at everything Neru can do, one control at a time.</p><p>Use <kbd>→</kbd> and <kbd>←</kbd> to move, <kbd>Esc</kbd> to skip. You can replay it from the <strong>⋯</strong> menu at the bottom of the sidebar.</p></> },
  { target: 'new-session', side: 'right', title: 'Sessions', body: <><p>Start a new session with <kbd>{mod}</kbd> <kbd>N</kbd>. Several can run at once.</p><p>From a project’s menu, start a <strong>worktree session</strong>: its own Git branch and folder, so parallel work never collides.</p></> },
  { target: 'team-nav', side: 'right', title: 'Team', body: <p>Claude Code, Codex, Cursor, OpenCode and Gemini in one shared thread, on the subscriptions you already have. It has its own walkthrough the first time you open it.</p> },
  { target: 'sessions', side: 'right', title: 'Projects and history', body: <p>Every project keeps its own sessions. Rename, delete or reveal them from the right-click menu; a spinner shows which ones are still working.</p> },
  { target: '.surface-switch', side: 'bottom', title: 'Chat or code', body: <p><strong>Chat</strong> is a plain conversation; your project is not sent. <strong>Code</strong> lets Neru read, search and edit the open project.</p> },
  { target: '.open-project-prompt', side: 'top', title: 'Open a project', body: <p>Open a folder or clone a repository. The terminal, files, changes and browser tools appear once a project is open.</p> },
  { target: '.titlebar-actions [aria-label="Command palette"]', side: 'bottom', title: 'Command palette', body: <p><kbd>{mod}</kbd> <kbd>K</kbd> finds any action, session, setting or file without leaving the keyboard.</p> },
  { target: '.titlebar-actions [aria-label="Terminal"]', side: 'bottom', title: 'Terminal', body: <p>Terminals inside Neru, in tabs and side by side. Neru can run commands here too, after you approve them.</p> },
  { target: '.titlebar-actions [aria-label="Files"]', side: 'bottom', title: 'Files', body: <p>Browse the project tree and preview files. Files Neru touched in this session are marked.</p> },
  { target: '.titlebar-actions [aria-label="Changes"]', side: 'bottom', title: 'Changes', body: <><p>Everything this session changed, as diffs. Leave line comments and send them back to Neru.</p><p>From <strong>⋮ → Source control</strong>: commit, push, open pull requests and follow CI checks, with auto-fix when they fail.</p></> },
  { target: '.titlebar-actions [aria-label="Review code"]', side: 'bottom', title: 'Review', body: <p>Ask Neru to review the current changes for bugs, risks and missing tests.</p> },
  { target: '.titlebar-actions [aria-label="Browser"]', side: 'bottom', title: 'Live preview', body: <p>Starts your dev server and opens the app in Neru’s own browser. Neru checks the page for errors itself.</p> },
  { target: '.home-input .neru-prompt', side: 'top', title: 'The message box', body: <ul><li><kbd>@</kbd> attaches project files</li><li><kbd>/</kbd> lists every command: <code>/model</code>, <code>/memory</code>, <code>/resume</code>, <code>/cost</code>…</li><li>Type while Neru works to <strong>steer</strong> it, or queue the next message</li><li>Paste or drop images straight in</li></ul> },
  { target: '.plus-trigger', side: 'top', title: 'Attach anything', body: <p>PDFs, Word, PowerPoint, Excel, images and project files.</p> },
  { target: '.mic-group', side: 'top', title: 'Dictation', body: <p>Speak instead of typing. A small model on this PC transcribes, so your voice never leaves it. The arrow picks the microphone and hold-to-talk.</p> },
  { target: '.composer-tools > .prompt-toggle', side: 'top', title: 'Web search', body: <p>Lets Neru search and read the web, and cite its sources.</p> },
  { target: '.composer-mode', side: 'top', title: 'Permission mode', body: <><p>How much Neru does on its own: <strong>Ask every time</strong>, <strong>Accept edits</strong>, <strong>Plan</strong> (read-only), <strong>Auto</strong> or <strong>Bypass</strong>.</p><p>Each approved edit saves a checkpoint, so you can undo it or rewind to before any message.</p></> },
  { target: '.composer-model | .composer-settings', side: 'top', title: 'Model', body: <p>Switch models any time; the list shows what your key can use once a provider is connected. If a model is slow, down or rate-limited, Neru falls back to the next best one you have a key for.</p> },
  { target: '.composer-effort', side: 'top', title: 'Effort', body: <p>How hard the model thinks before answering, on models that support it.</p> },
  { target: '.context-meter', side: 'top', title: 'Context', body: <p>How full the conversation is. Long sessions compact on their own so they stay healthy.</p> },
  { target: 'settings', side: 'right', title: 'Settings', body: <><p><kbd>{mod}</kbd> <kbd>,</kbd> opens them:</p><ul><li><strong>Model</strong>: providers and keys, each saved for when you switch back</li><li><strong>Connectors</strong>: Linear, Notion, GitHub and more over MCP</li><li><strong>Skills</strong>, <strong>Agents</strong> and <strong>Imports</strong></li><li><strong>Voice</strong> and <strong>Appearance</strong></li></ul></> },
  { title: 'There’s more under the hood', body: <ul><li><strong>Memory</strong> keeps your preferences across sessions (<code>/memory</code>)</li><li><strong>Hooks</strong> run your scripts from <code>.neru/hooks.json</code></li><li><strong>Sub-agents</strong> explore big codebases in parallel</li><li><strong>neru</strong> in any terminal shares sessions with the app</li></ul> },
]
