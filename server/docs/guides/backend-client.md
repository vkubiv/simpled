# backend-client guide

`@simpled/backend-client` (`server/backend-client`) is the TypeScript client for main-svc. The web app, the e2e
suite and later the simpled CLI use it; nothing else talks to main-svc. Adapted from hobby-pro's
`backend-client-guide.md`.

```
web-app  ─┐
e2e      ─┼─> @simpled/backend-client ─> main-svc
CLI      ─┘
```

## Principles

1. **Workspace source, no build.** `package.json` `exports` points at `src/index.ts`; Vite and Vitest transpile it
   through the workspace link. Its only runtime dependency is `zod`.
2. **Stateless client, explicit caller.** `MainClient` holds configuration. Every method takes an `ApiContext`
   first, so one client serves many callers (one e2e test acts as an owner and a member at once).
3. **Typed expected errors.** Every method returns `Promise<Result<T, E>>` with `E` a union of error classes.
   Unexpected statuses throw.
4. **One job per layer.** `transport.ts` (HTTP → transport errors) → `api/*` (endpoints and error translation) →
   `model/*` (Zod schemas) → `clients/main.ts` (the facade).

## Layout

```
src/
  index.ts          the only entry point: re-exports everything below except Transport
  result.ts         Result, ok, fail, hasError, getError, unwrap, forward, describeErrors
  errors.ts         transport errors, ErrorCode, domain errors
  context.ts        ApiContext, anonymous, browser, withToken, MemoryCookieJar
  transport.ts      fetch wrapper
  model/            common (parse, isoDate, OrgRole), me, organization, invitation, team, project
  api/              common (signedIn, rateLimited, asForbidden, asFormField), auth, me, orgs, invitations, teams, projects
  clients/main.ts   MainClient: auth, me, orgs, invitations, teams, projects
```

## Contexts

| Caller | Context | Session |
|---|---|---|
| Web app | `browser` | the browser's HttpOnly `session` cookie (same origin, `/api`) |
| e2e, Node scripts | `{ cookieJar: new MemoryCookieJar() }` | cookies kept in memory, dropped when expired or emptied |
| CLI | `withToken(token)` from `auth.loginWithToken` | bearer header |
| Pre-auth calls | `anonymous` | none |

## Transport

`Transport.request` maps main-svc's generic contract and nothing else:

| Response | Result |
|---|---|
| 2xx | `ok(body)` |
| 401 | `ApiNotAuthorized` |
| 429 | `RateLimited` |
| 400 with `[{code, incorrectValue, details}]` | one `ValidationError` per entry |
| anything else | throws, with the request and response bodies in the message |

A failed `fetch` (offline, refused) throws too. `TransportConfig.fetch` lets the web app wrap fetch
(`sessionFetch` signs the app out on any 401).

## Errors

`TransportError = ApiNotAuthorized | RateLimited | ValidationError`. API methods translate `ValidationError`s by
`code` and `incorrectValue` (the field or route parameter main-svc named) into domain errors whose `message` is
the sentence to show:

| Error | From |
|---|---|
| `InvalidFormField(fieldName, details)` | any field-level refusal on a form |
| `InvalidCredentials` | 401 on login |
| `EmailNotConfirmed` | `email-not-confirmed` on login |
| `AccountAlreadyExists` | `already-exist` on `email` when registering |
| `InvalidLink` | an unusable emailed token (`token`) |
| `OrganizationNotFound` / `TeamNotFound` / `ProjectNotFound` / `MemberNotFound` / `InvitationNotFound` | `doesnt-exist` on `orgId` / `teamId` / `projectId` / `accountId` / `invitationId` |
| `RoleForbidden(reason, fieldName)` | `forbidden` |
| `LastOwner` | `incorrect-state` on `accountId` |
| `AlreadyMember` | already in the organization or team, or a team already linked |
| `InvitationForAnotherEmail` | accepting while signed in as another email |

Add a class when a caller reacts to an outcome differently; otherwise map to `InvalidFormField` or let the
mapper throw.

## Writing a method

```ts
/** Adds an organization member to the team. */
async addMember(
  ctx: ApiContext,
  teamId: number,
  accountId: string,
): Promise<Result<void, ApiNotAuthorized | TeamNotFound | MemberNotFound | AlreadyMember | RoleForbidden>> {
  const r = await this.t.post(ctx, `/Teams/${teamId}/Members/${accountId}`);
  return forward(
    r,
    () => undefined,
    (e) => {
      if (e instanceof ValidationError && e.incorrectValue === "accountId") {
        return e.code === ErrorCode.AlreadyExist ? new AlreadyMember(e.details, e) : new MemberNotFound(accountId, e);
      }
      return manageFailure(teamId)(e);
    },
  );
}
```

- `ctx` first; the return type lists every error a caller may meet.
- The failure mapper handles the `ValidationError`s it expects, then ends in `signedIn(e)` (keeps
  `ApiNotAuthorized`, throws the rest), `rateLimited(e)` for anonymous rate-limited endpoints, or
  `throw new UnexpectedApiError(e)`. Adding a transport error then breaks the build wherever it is not handled.
- Write one mapper per method, or a helper with a precise return type (`manageFailure`). A shared option-bag
  mapper widens every method's union to all errors.
- Parse success bodies with `parse(Schema, json)`; a mismatch is a `ParsingError` naming the field.
- Tokens from emailed links go in request bodies.

## Models

Zod schemas named like their type: `export const Team = z.object({...}); export type Team = z.output<typeof Team>`.
Dates are `isoDate` (string on the wire, `Date` in the model), roles `OrgRole`. Request shapes the caller builds
are plain interfaces (`Registration`, `NewProject`, `ProjectUpdate`).

## Adding an endpoint

1. The model in `src/model/` (new file → export it from `index.ts`).
2. The method in the matching `src/api/` group (a new group → a field on `MainClient` and an export).
3. New domain errors in `errors.ts`.
4. `just check-ts`, then a scenario in `e2e` ([e2e.md](e2e.md)).
