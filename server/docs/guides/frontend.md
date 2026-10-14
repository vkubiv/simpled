# web-app guide

`server/web-app`: React 19 + Vite + Tailwind v4 on the subtree.js architecture (`subtree.js`, `trunk.js`,
`operation-result.js`), talking to main-svc only through `@simpled/backend-client`. Adapted from hobby-pro's
frontend; the libraries' own references are in `D:\apps\subtree.js\docs` (`subtree.md`, `operation-result.md`,
`ARCHITECTURE.md`).

## Running

```sh
# from server/
just infra                   # terminal 1: Postgres, mailer, gateway (run main-svc from the IDE)
just web-dev                 # terminal 2: Vite on :7000. Open http://localhost:7080: emailed links land there.
just run                     # the built image (nginx) behind the gateway, http://localhost:7080
just check-ts                # tsc -b, biome, vitest across the workspace
cd web-app && npm test       # the controller tests alone
```

## Layout

```
src/
  main.tsx             AccountSession(browser), MainClient("/api", sessionFetch), createAppDeps, BrowserRouter
  app.tsx              routes and gates: signed-out pages, Protected, OrgArea (the shell + organization pages)
  app-controller.ts    the gate (loading | failed | signedOut | signedIn) and `home`
  app-deps.ts          AppDeps: client, session, authHandler, meRepository, orgRepository
  core/                no React
    auth/              AccountSession, sessionFetch, createAuthHandler
    ops/               auth, org, invitation, team, project, monitoring
    repositories/      MeRepository (account + organizations), OrgRepository (the organization shown)
    primitives/        errors, forms (validators), load-status, roles, routes (paths), time (timeAgo)
  shell/               ShellController + ShellLayout: org switcher, nav, not-found
  pages/<page>/        <page>-model.ts, -controller.ts, -page.tsx, -route.tsx, -controller.test.ts
  components/ui.tsx    presentational pieces: AuthCard, Field, Notice, RoleBadge, Section, ...
  testing/fakes.ts     fakeClient, makeDeps, signedInDeps, model factories, flush
```

Pages: `sign-in`, `register`, `check-email`, `confirm-email`, `forgot-password`, `reset-password`, `invitation`,
`new-org`, and under `/orgs/:orgId`: `org-overview`, `projects`, `project`, `teams`, `team`, `members`, `github`,
`org-settings`. `project-deployments` is not a route: `ProjectRoute` renders its own `<Subtree>` into the project
page's `deployments` slot, and its controller re-reads the statuses every 15 s until disposed.

## Session

main-svc's session is an HttpOnly cookie the app cannot read, so `AccountSession.stage` is what the last answer
said: `restoreSession` asks `/Me` on start, signing in and confirming an email restore again, signing out sets
`signedOut`, and `sessionFetch` turns any 401 into `signedOut` (the gate then shows sign-in with `?next=`).
`AppController` forgets the account and the organization on the change to signed out, never while signed out: a
sign-in in progress writes the account before the stage changes. The e2e suite runs the same code with a
`MemoryCookieJar` context.

## Core layer

- **Repositories** extend `ChangeNotifier`, expose getters, change through methods that notify, hold no logic.
- **Ops** are exported functions taking one options object; every dependency is a named field
  (`{ client, authHandler, orgId }`). They return `AsyncResult<T, E>` when the caller handles a failure,
  `Promise<void>` when they report through a repository. Only ops change repositories. Authenticated calls go
  through `authHandler.callApi((ctx) => client.x.y(ctx, ...))`; the ones that handle their own 401 (restoring,
  signing in, anonymous calls) use `session.context`.
- After a change that alters the account's organizations or roles, ops call `refreshMe`; after one that alters
  the shown organization, `loadOrg` or `orgRepository.set`.

## UI layer

- **Model.** `State` is a class of `Rx` / `RxList` fields; `Actions` an abstract class (also the token). Drafts
  are plain objects replaced on change: `draft.update((d) => ({ ...d, ...patch }))`.
- **Controller.** `Deps = Pick<AppDeps, ...>`, a `Routing` of callbacks, and the controller, which puts `state`
  and itself into its subtree. No business logic: call ops. Mirror repositories with `sync`; derive state with
  `sync` over the `Rx` fields read. Actions are arrow properties; after an `await` check `this.isDisposed` before
  writing or navigating. No DOM in a controller: the e2e suite builds them in Node.
- **Permissions.** Pages read `orgRepository.org.permissions` (and `canManage`, `canChangeRole`, ... on items)
  and hide what they deny. main-svc enforces them anyway.
- **View.** `useSubtree(Token)`, `useObserver()` once, `observer.watch(rx)`. Views never touch repositories, ops
  or the router, except `<Link>` / `<NavLink>`. UI-only state (a select's pending choice) may stay in `useState`;
  `window.confirm` stays in the view.
- **Route component.** Reads `useParams` / `useSearchParams` / `useNavigate` once, builds the controller with
  `pick(deps, ...)` and a routing object, and renders `<Subtree key="<page>" deps={[ids]}>`. `app.tsx` only maps
  paths. Paths are built with `core/primitives/routes.ts` `paths`.
- **Forms.** Validate in the controller with `core/primitives/forms.ts` before calling; then put the backend's
  `InvalidFormField`s on their fields with `fieldErrors(r.errors)` and the rest above the form with
  `errorText(formLevelErrors(r.errors))`.
- **Styling.** Tailwind with the tokens and component classes in `index.css` (`panel`, `btn btn-primary`, `field`,
  `label`, `note note-danger`, `badge`, `link`). New colours go in `@theme` as tokens.

## Emailed links

`confirm-email?token=`, `reset-password?token=` and `invitation?token=` are pages of this app (main-svc builds
them from `WEB_APP_URL`). Confirming takes a click instead of happening on load: mail scanners open links before
people do, and the token works once. nginx keeps these paths out of its access log and sends no Referer.

## Tests

Controller tests (`*.test.ts`, Vitest, Node, no DOM): build deps with `makeDeps(fakeClient({...}), stage)` or
`signedInDeps(client, makeOrg(role))`, drive actions, assert `state.x.value` and routing spies. `sync` runs its
first pass in a microtask: `await flush()` before asserting on mirrored state. Results come from the client's
`ok` / `fail` with its error classes. The same controllers run against the stack in `e2e/test/web-app/`
([e2e.md](e2e.md)).

## Adding a page

1. Ops for its calls in `core/ops/` (and the client method, if new).
2. `pages/<page>/`: model, controller (`Deps`, `Routing`), page, route, controller test.
3. A path in `routes.ts`, the route in `app.tsx` (inside `OrgArea` for an organization page), a nav entry in
   `shell-layout.tsx` if it has one.
4. `just check-ts`; an e2e case in `e2e/test/web-app/` when it changes what the app does with real answers.

## Do not

- Read or write `.value` in a view, or subscribe by hand.
- Put logic in repositories or controllers, or change a repository from a controller.
- Import the router in a controller, or `useParams` in a page component.
- Mutate `Rx` values in place.
- Add React Query, React context for app state, a form library or a DI container.
- Read a parent `<Subtree>`'s tokens from a child one: pass what a page needs through its deps.
