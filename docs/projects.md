# Projects and chat threads in Cloud Bee

Use **+ Project** in the sidebar to set a name, colour, optional description and
an existing local Windows workspace folder. Leaving the folder empty uses Core's
configured workspace. Projects and their chat memberships are saved by Core.

Select a project to filter its chats. The **+** beside its name creates a chat
inside that project. The main **New chat** button also lets you choose a project,
title and conversation type:

- **Council**: one or more models in a shared conversation, with the existing
  model selectors, microtasks and progress panel. Each Council chat has its own
  saved history and draft.
- **Chat / Work**: an agent conversation, optionally with additional model tabs.
  Normal Chat needs an API/local connection. Existing CLI subscription sign-ins
  are available through Council. Work still requires its explicit authorization.

A project can contain multiple threads of either type. Switch between them from
the sidebar. **Organize selected** renames, pins, archives or moves a chat to
another project. **All chats** and **Unassigned** keep older conversations
accessible. Projects are not inferred from matching folder names.

The **...** beside a project opens its settings. Folder changes apply to new
threads; existing threads keep their original workspace and permissions. Moving
a chat between projects changes its organization only. Archiving a project keeps
its chats and files; enable **Archived projects** to edit and restore it.

Project folders must be existing ordinary local directories. Network shares and
folder links are unsupported. Choosing a folder creates no model request and
grants no execution or file-reading permission. Council starts with discussion
access; use a new Council to choose shared read or isolated Work access.

Core persists project settings, revision checks and retry receipts in its private
extension state. No SQL migration or additional service is required. Back up the
Core state while stopped. API entry points are `GET /v1/bee/projects`,
`POST /v1/bee/projects/{id}` and `POST /v1/bee/projects/threads/{id}`.

Validation uses disposable fixture runtimes: `tests/windows_projects.py` checks
membership, local workspaces, explicit Work authorization, revisions, retries and
restart; `tests/windows_gui.py` checks the WPF project and multiple-thread flows.
